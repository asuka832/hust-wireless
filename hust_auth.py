"""HUST wired campus network authentication, using only the Python standard library."""

from __future__ import annotations

import base64
import ctypes
from ctypes import wintypes
import json
import os
from pathlib import Path
from queue import Empty, Queue
import sys
import time
from threading import Lock, Thread
from urllib.error import HTTPError, URLError
from urllib.parse import parse_qs, quote, urlsplit
from urllib.request import HTTPRedirectHandler, ProxyHandler, Request, build_opener


PORTAL_URL = "http://172.18.18.61:8080/eportal/"
PROBES = (
    ("http://connect.rom.miui.com/generate_204", 204),
    ("http://connectivitycheck.platform.hicloud.com/generate_204", 204),
    ("http://123.123.123.123/", None),  # The portal's own expired-session redirect target.
)
NETWORK_CHECK_TIMEOUT = 3.0
PROBE_TIMEOUT = 2.0
_ACTIVE_PROBES: dict[tuple[str, str], Thread] = {}
_PROBE_LOCK = Lock()
RUN_KEY = r"Software\Microsoft\Windows\CurrentVersion\Run"
RUN_NAME = "HUSTWiredAuth"


class PortalError(Exception):
    pass


class LoginUrlRequired(PortalError):
    pass


class NoRedirect(HTTPRedirectHandler):
    def redirect_request(self, request, fp, code, msg, headers, newurl):
        return None


def encode_component(value: str) -> str:
    return quote(value, safe="~!*'()-._")


def double_encode(value: str) -> str:
    return encode_component(encode_component(value))


def rsa_encrypt(password: str, mac: str, exponent_hex: str, modulus_hex: str) -> str:
    """Match the portal's RSAUtils.encryptedString for UTF-16 code units."""
    try:
        exponent, modulus = int(exponent_hex, 16), int(modulus_hex, 16)
    except (TypeError, ValueError) as exc:
        raise PortalError("认证页返回了无效的 RSA 公钥") from exc
    digits = (modulus.bit_length() + 15) // 16
    chunk_size = 2 * (digits - 1)
    if exponent <= 0 or modulus <= 0 or chunk_size <= 0:
        raise PortalError("认证页返回了无效的 RSA 公钥")
    source = (password + ">" + mac).encode("utf-16-le", "surrogatepass")
    units = [int.from_bytes(source[i:i + 2], "little") for i in range(0, len(source), 2)]
    units.reverse()  # JavaScript split('').reverse().join('') reverses UTF-16 units.
    result = []
    for start in range(0, len(units), chunk_size):
        block = units[start:start + chunk_size]
        block.extend([0] * (chunk_size - len(block)))
        value = sum((block[i] + (block[i + 1] << 8)) << (8 * i)
                    for i in range(0, chunk_size, 2))
        encrypted = pow(value, exponent, modulus)
        raw_hex = format(encrypted, "x")
        result.append(raw_hex.zfill(max(4, (len(raw_hex) + 3) // 4 * 4)))
    return " ".join(result)


class Portal:
    def __init__(self, base_url: str = PORTAL_URL):
        parts = urlsplit(base_url)
        if parts.scheme != "http" or not parts.netloc:
            raise PortalError("认证服务器地址必须是 HTTP URL")
        self.base_url = base_url.rstrip("/") + "/"
        self.opener = build_opener(ProxyHandler({}), NoRedirect())

    def _post(self, method: str, values: dict[str, str]) -> dict:
        body = "&".join(f"{key}={value}" for key, value in values.items()).encode("ascii")
        request = Request(
            self.base_url + "InterFace.do?method=" + method,
            data=body,
            headers={"Content-Type": "application/x-www-form-urlencoded; charset=UTF-8"},
        )
        try:
            with self.opener.open(request, timeout=6) as response:
                if response.status != 200:
                    raise PortalError(f"认证服务器返回 HTTP {response.status}")
                return json.load(response)
        except (URLError, TimeoutError) as exc:
            raise PortalError(f"无法连接认证服务器：{exc.reason if isinstance(exc, URLError) else exc}") from exc
        except (ValueError, UnicodeError) as exc:
            raise PortalError("认证服务器返回了无法解析的数据") from exc

    def login(self, username: str, password: str, query_string: str, service: str = "") -> dict:
        page = self._post("pageInfo", {"queryString": encode_component(query_string)})
        if page.get("validCodeUrl"):
            raise PortalError("当前需要验证码，请用网页登录")
        if page.get("prefixName") == "true" and page.get("prefixValue"):
            username = page["prefixValue"] + username
        if page.get("passwordEncrypt") != "true":
            raise PortalError("认证页未启用密码加密；为避免通过 HTTP 发送明文密码，已停止登录")
        mac = next((part[4:] for part in query_string.split("&") if part.lower().startswith("mac=")), "")
        password = rsa_encrypt(password, mac or "111111111",
                               page.get("publicKeyExponent", ""),
                               page.get("publicKeyModulus", ""))
        values = {
            "userId": double_encode(username),
            "password": double_encode(password),
            "service": double_encode(service),
            "queryString": double_encode(query_string),
            "operatorPwd": "",
            "operatorUserId": "",
            "validcode": "",
            "passwordEncrypt": double_encode("true"),
        }
        result = self._post("login", values)
        if result.get("result") != "success":
            raise PortalError("认证失败：" + str(result.get("message") or result.get("result") or "未知原因"))
        return result

    def _probe_url(self, url: str, expected: bytes | int | None,
                   portal_page: bool) -> tuple[str, str] | None:
        # Each worker owns its opener; DNS and response reads stay off the menu thread.
        opener = build_opener(ProxyHandler({}), NoRedirect())
        request = Request(url, headers={"User-Agent": "HUST-Wired-Auth/1.0"})
        try:
            with opener.open(request, timeout=PROBE_TIMEOUT) as response:
                if isinstance(expected, int) and response.status == expected:
                    return "online", ""
                page = urlsplit(response.url)
                if portal_page and response.status == 200:
                    query = page.query
                    return ("need_auth", query) if query else ("missing_query", "")
                body = response.read(2048)
                if isinstance(expected, bytes) and expected in body:
                    return "online", ""
                if (b"eportal" in body.lower() and page.query
                        and page.hostname == urlsplit(self.base_url).hostname):
                    return "need_auth", page.query
        except HTTPError as exc:
            try:
                if exc.code in (301, 302, 303, 307, 308):
                    page = urlsplit(exc.headers.get("Location", ""))
                    if page.hostname == urlsplit(self.base_url).hostname:
                        return ("need_auth", page.query) if page.query else ("missing_query", "")
            except ValueError:
                pass
            finally:
                exc.close()
        except (OSError, ValueError):
            pass
        return None

    def check_network(self) -> tuple[str, str]:
        """Probe concurrently, returning within the total budget even if DNS stalls."""
        deadline = time.monotonic() + NETWORK_CHECK_TIMEOUT
        results: Queue = Queue()

        def probe(url, expected, portal_page):
            results.put(self._probe_url(url, expected, portal_page))

        targets = [(url, expected, False) for url, expected in PROBES]
        targets.append((self.base_url + "index.jsp", None, True))
        pending = 0
        with _PROBE_LOCK:
            for key, worker in list(_ACTIVE_PROBES.items()):
                if not worker.is_alive():
                    del _ACTIVE_PROBES[key]
            for url, expected, portal_page in targets:
                key = (self.base_url, url)
                if key in _ACTIVE_PROBES:
                    continue  # Do not accumulate workers when OS DNS remains stuck.
                worker = Thread(target=probe, args=(url, expected, portal_page), daemon=True)
                _ACTIVE_PROBES[key] = worker
                worker.start()
                pending += 1
        fallback = ("unreachable", "")
        while pending:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                break
            try:
                result = results.get(timeout=remaining)
            except Empty:
                break
            pending -= 1
            if result is not None:
                if result[0] in ("online", "need_auth"):
                    return result
                fallback = result
        return fallback


def config_path() -> Path:
    return Path(os.environ.get("APPDATA", str(Path.home()))) / "hust-wireless-cli" / "config.json"


def login_url_path() -> Path:
    return config_path().with_name("wired-login-url.txt")


def legacy_login_url_path() -> Path:
    return Path(__file__).resolve().with_name("wired-login-url.txt")


def parse_login_url(url: str, portal_base: str) -> str:
    try:
        page = urlsplit(url.strip())
    except ValueError as exc:
        raise PortalError("网址格式无效，请重新复制浏览器地址栏的完整地址") from exc
    portal = urlsplit(portal_base)
    expected_path = portal.path.rstrip("/") + "/index.jsp"
    if (page.scheme, page.netloc, page.path) != (portal.scheme, portal.netloc, expected_path):
        raise PortalError("保存的地址不是 index.jsp 登录页；请复制绿色登录按钮出现时的地址")
    params = parse_qs(page.query)
    if not params.get("nasip") or not params.get("wlanuserip"):
        raise PortalError("登录页地址缺少 nasip 或 wlanuserip 认证参数")
    return page.query


def save_login_url(url: str, portal_base: str) -> str:
    url = url.strip()
    query = parse_login_url(url, portal_base)
    path = login_url_path()
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(url, encoding="utf-8")
    return query


def load_login_query(portal_base: str) -> str:
    for path in (login_url_path(), legacy_login_url_path()):
        try:
            url = path.read_text(encoding="utf-8-sig").strip()
        except FileNotFoundError:
            continue
        try:
            query = parse_login_url(url, portal_base)
        except PortalError as exc:
            raise LoginUrlRequired("未取得认证参数：本地登录地址无效；请选择菜单 9 更新登录地址") from exc
        if path != login_url_path():
            save_login_url(url, portal_base)
        return query
    raise LoginUrlRequired("未取得认证参数；请打开菜单 9 粘贴自己的 index.jsp 登录页地址")


def prompt_login_url(portal_base: str) -> bool:
    print("请在待认证状态打开你平时使用的校园网认证网页，停在绿色“连接 Login”按钮出现的页面。")
    print("复制浏览器地址栏中的完整 index.jsp? 网址，粘贴到下面；不要复制登录成功页，也不要发送到聊天或 Issue。")
    while True:
        url = input("登录页网址（直接回车取消）：").strip()
        if not url:
            print("已取消，原有登录地址保持不变。")
            return False
        try:
            save_login_url(url, portal_base)
        except PortalError as exc:
            print(str(exc))
            continue
        print("已保存到当前 Windows 用户的配置目录，下次会自动复用。")
        return True


def _dpapi(data: bytes, protect: bool) -> bytes:
    if os.name != "nt":
        raise PortalError("密码加密需要 Windows")

    class DataBlob(ctypes.Structure):
        _fields_ = [("cbData", wintypes.DWORD), ("pbData", ctypes.POINTER(ctypes.c_ubyte))]

    buffer = (ctypes.c_ubyte * len(data)).from_buffer_copy(data)
    input_blob = DataBlob(len(data), buffer)
    output_blob = DataBlob()
    crypt32 = ctypes.WinDLL("crypt32", use_last_error=True)
    func = crypt32.CryptProtectData if protect else crypt32.CryptUnprotectData
    func.restype = wintypes.BOOL
    func.argtypes = ([ctypes.POINTER(DataBlob), wintypes.LPCWSTR, ctypes.POINTER(DataBlob),
                      ctypes.c_void_p, ctypes.c_void_p, wintypes.DWORD, ctypes.POINTER(DataBlob)]
                     if protect else
                     [ctypes.POINTER(DataBlob), ctypes.c_void_p, ctypes.POINTER(DataBlob),
                      ctypes.c_void_p, ctypes.c_void_p, wintypes.DWORD, ctypes.POINTER(DataBlob)])
    args = (ctypes.byref(input_blob), "HUST wired auth" if protect else None, None,
            None, None, 0, ctypes.byref(output_blob))
    if not func(*args):
        raise PortalError(f"Windows 密码保护失败，错误码 {ctypes.get_last_error()}")
    try:
        return ctypes.string_at(output_blob.pbData, output_blob.cbData)
    finally:
        ctypes.WinDLL("kernel32").LocalFree(ctypes.cast(output_blob.pbData, ctypes.c_void_p))


def save_credentials(username: str, password: str, service: str = "") -> None:
    path = config_path()
    path.parent.mkdir(parents=True, exist_ok=True)
    protected = base64.b64encode(_dpapi(password.encode("utf-8"), True)).decode("ascii")
    path.write_text(json.dumps({"username": username, "password_dpapi": protected,
                                "service": service}, ensure_ascii=False, indent=2), encoding="utf-8")


def load_credentials() -> tuple[str, str, str]:
    try:
        data = json.loads(config_path().read_text(encoding="utf-8"))
        password = _dpapi(base64.b64decode(data["password_dpapi"]), False).decode("utf-8")
        return data["username"], password, data.get("service", "")
    except FileNotFoundError as exc:
        raise PortalError("尚未设置账号，请先选择菜单 4 设置账号") from exc
    except (KeyError, ValueError, UnicodeError) as exc:
        raise PortalError("账号配置已损坏，请重新运行 setup") from exc


def autostart_args() -> list[str]:
    pythonw = Path(sys.executable).with_name("pythonw.exe")
    return [str(pythonw), "-I", "-S", str(Path(__file__).resolve()), "watch"]


def install_autostart() -> None:
    import subprocess
    import winreg

    load_credentials()
    pythonw = Path(sys.executable).with_name("pythonw.exe")
    if not pythonw.exists():
        raise PortalError("未找到 pythonw.exe，无法安装开机自启")
    command = subprocess.list2cmdline(autostart_args())
    with winreg.CreateKey(winreg.HKEY_CURRENT_USER, RUN_KEY) as key:
        winreg.SetValueEx(key, RUN_NAME, 0, winreg.REG_SZ, command)


def uninstall_autostart() -> None:
    import winreg
    try:
        with winreg.OpenKey(winreg.HKEY_CURRENT_USER, RUN_KEY, 0, winreg.KEY_SET_VALUE) as key:
            winreg.DeleteValue(key, RUN_NAME)
    except FileNotFoundError:
        pass


def remove_tool_data() -> None:
    """Remove this portable tool's startup entry and known local user files."""
    uninstall_autostart()
    config = config_path()
    config.unlink(missing_ok=True)  # An active watcher sees this and exits.
    for path in {login_url_path(), legacy_login_url_path()}:
        path.unlink(missing_ok=True)
    log_path = config.with_name("hust-auth.log")
    for attempt in range(30):
        try:
            log_path.unlink(missing_ok=True)
            break
        except PermissionError:
            if attempt == 29:
                raise PortalError("后台监测仍占用日志文件；请关闭监测窗口后重试卸载")
            if attempt == 0:
                print("正在等待后台监测退出…")
            time.sleep(1)
    try:
        config.parent.rmdir()
    except OSError:
        pass  # Leave the directory if it contains unrelated files.


def login_once(portal: Portal, credentials: tuple[str, str, str]) -> str:
    state, query = portal.check_network()
    if state == "online":
        return "已经联网，无需认证"
    if state == "unreachable":
        raise PortalError("尚未连接校园有线网，或认证服务器不可达")
    if state == "missing_query":
        query = load_login_query(portal.base_url)
    portal.login(credentials[0], credentials[1], query, credentials[2])
    if state == "need_auth":
        url = portal.base_url + "index.jsp?" + query
        try:
            parse_login_url(url, portal.base_url)
        except PortalError:
            pass
        else:
            try:
                save_login_url(url, portal.base_url)
            except OSError:
                return "认证服务器已接受登录，但本地地址保存失败；可通过菜单 9 重新保存"
    return "认证服务器已接受登录"


def login_with_guidance(portal: Portal, credentials: tuple[str, str, str]) -> str:
    try:
        return login_once(portal, credentials)
    except LoginUrlRequired as exc:
        print(str(exc))
        if not prompt_login_url(portal.base_url):
            return "本次登录已取消。"
        return login_once(portal, credentials)


def append_log(path: Path, level: str, message: str) -> None:
    now = time.time()
    timestamp = time.strftime("%Y-%m-%d %H:%M:%S", time.localtime(now))
    try:
        with path.open("a", encoding="utf-8") as stream:
            stream.write(f"{timestamp},{int(now % 1 * 1000):03d} {level} {message}\n")
    except OSError:
        pass  # Authentication should continue even if the log becomes unavailable.


def watch(portal: Portal, credentials: tuple[str, str, str]) -> None:
    if not config_path().exists():
        return
    log_path = config_path().with_name("hust-auth.log")
    failures = 0
    url_notice_logged = False
    while config_path().exists():
        try:
            message = login_once(portal, credentials)
            if message != "已经联网，无需认证":
                append_log(log_path, "INFO", message)
            failures = 0
            url_notice_logged = False
        except LoginUrlRequired as exc:
            if not url_notice_logged:
                append_log(log_path, "WARNING", str(exc))
                url_notice_logged = True
            failures = 0
        except PortalError as exc:
            if "尚未连接校园有线网" in str(exc) or "未取得认证参数" in str(exc):
                failures = 0
            else:
                failures += 1
                append_log(log_path, "ERROR", str(exc))
                if failures >= 5:
                    append_log(log_path, "ERROR", "连续认证失败 5 次，监测已停止；请检查账号后重新启动")
                    return
        delay = 20 if failures == 0 else min(20 * 2 ** failures, 300)
        for _ in range(delay):
            if not config_path().exists():
                return
            time.sleep(1)


MENU_OPTIONS = {
    "1": ("login", "立即登录"),
    "2": ("status", "检查网络"),
    "3": ("diagnose", "参数诊断"),
    "4": ("setup", "设置账号"),
    "5": ("install", "启用开机自启"),
    "6": ("uninstall", "取消开机自启"),
    "7": ("watch", "持续监测"),
    "8": ("remove", "卸载工具"),
    "9": ("set-url", "更新登录地址"),
}


def terminal_width(value: str) -> int:
    import unicodedata

    return sum(2 if unicodedata.east_asian_width(char) in ("F", "W") else 1 for char in value)


def menu_lines(width: int) -> list[str]:
    items = [f"[{number}] {label}" for number, (_, label) in MENU_OPTIONS.items()]
    items.append("[0] 退出")
    cell_width = max(terminal_width(item) for item in items) + 4
    columns = min(3, max(1, width // cell_width))
    lines = []
    for offset in range(0, len(items), columns):
        row = items[offset:offset + columns]
        lines.append("  " + "".join(item + " " * (cell_width - terminal_width(item))
                                    for item in row).rstrip())
    return lines


def confirm_action(label: str) -> bool:
    while True:
        answer = input(f"确定{label}？(y/n)：").strip().lower()
        if answer == "y":
            return True
        if answer == "n":
            return False
        print("请输入 y 或 n。")


def run_menu() -> int:
    while True:
        print("\n华科有线网认证")
        try:
            width = os.get_terminal_size().columns
        except OSError:
            width = 80
        print("\n".join(menu_lines(width)))
        print("持续监测时按 Ctrl+C 返回菜单。")
        try:
            choice = input("请选择数字：").strip()
        except (EOFError, KeyboardInterrupt):
            print("\n已退出。")
            return 0
        if choice == "0":
            return 0
        option = MENU_OPTIONS.get(choice)
        if option is None:
            print("无效选项，请输入菜单中的数字。")
            continue
        command, _ = option
        try:
            main([command], interactive=True)
        except (EOFError, KeyboardInterrupt):
            print("\n已停止当前操作，返回菜单。")


def main(argv: list[str] | None = None, *, interactive: bool | None = None) -> int:
    import argparse

    parser = argparse.ArgumentParser(description="华科有线网命令行自动认证")
    parser.add_argument("command", nargs="?", choices=["setup", "login", "status", "diagnose", "watch", "install", "uninstall", "remove", "set-url"])
    args = parser.parse_args(argv)
    if args.command is None:
        return run_menu()
    if interactive is None:
        interactive = sys.stdin is not None and sys.stdin.isatty()
    try:
        if args.command == "setup":
            print("安全提示：认证服务器使用 HTTP，无法验证公钥来源；请先阅读 README 的安全与已知限制。")
            username = input("校园网账号：").strip()
            if not username:
                raise PortalError("账号不能为空")
            import getpass

            password = getpass.getpass("校园网密码：")
            if not password:
                raise PortalError("密码不能为空")
            service = input("运营商代码（通常直接回车）：").strip()
            save_credentials(username, password, service)
            print("已保存账号。需要开机自动认证时，请选择 5 启用开机自启。")
        elif args.command == "install":
            install_autostart()
            print("已设置登录 Windows 后自动监测。")
        elif args.command == "uninstall":
            uninstall_autostart()
            print("已取消开机自启。")
        elif args.command == "set-url":
            prompt_login_url(PORTAL_URL)
        elif args.command == "remove":
            print("将清除开机自启、账号配置、日志和本地登录地址。项目源码会保留。")
            if confirm_action("卸载工具"):
                remove_tool_data()
                print("本地数据已清理；删除项目文件夹即可移除程序文件。")
            else:
                print("已取消。")
        else:
            portal = Portal()
            if args.command in ("status", "diagnose"):
                print(f"正在检测网络（最长约 {NETWORK_CHECK_TIMEOUT:g} 秒，Ctrl+C 取消）…", flush=True)
                started = time.monotonic()
                state, query = portal.check_network()
                message = {"online": "已经联网", "need_auth": "需要认证", "missing_query": "需要认证，但未取得认证参数", "unreachable": "检测超时或网络不可达"}[state]
                elapsed_ms = round((time.monotonic() - started) * 1000)
                print(f"{message}（延迟 {elapsed_ms} ms）")
                if args.command == "diagnose":
                    print("认证参数字段：" + (", ".join(sorted(parse_qs(query).keys())) if query else "无"))
                    try:
                        saved_query = load_login_query(portal.base_url)
                        print("本地登录地址字段：" + ", ".join(sorted(parse_qs(saved_query).keys())))
                    except PortalError as exc:
                        print("本地登录地址：" + str(exc))
            else:
                credentials = load_credentials()
                if args.command == "login":
                    if interactive:
                        print("正在检查网络并尝试认证…", flush=True)
                    login = login_with_guidance if interactive else login_once
                    print(login(portal, credentials))
                else:
                    watch(portal, credentials)
        return 0
    except (EOFError, KeyboardInterrupt):
        print("\n已取消当前操作。")
        return 0
    except (PortalError, OSError) as exc:
        print(f"错误：{exc}", file=sys.stderr)
        if args.command == "login" and str(exc).startswith("认证失败"):
            print("若设备或网络地址已变化，请选择菜单 9 更新登录地址后再试；也请检查账号密码。", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
