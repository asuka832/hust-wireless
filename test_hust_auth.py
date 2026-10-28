import json
import io
import os
from pathlib import Path
import tempfile
import time
import unittest
from contextlib import redirect_stderr, redirect_stdout
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from threading import Event, Thread
from urllib.parse import parse_qs
from unittest.mock import patch

import hust_auth


class PortalHandler(BaseHTTPRequestHandler):
    requests = []

    def do_POST(self):
        body = self.rfile.read(int(self.headers["Content-Length"])).decode()
        self.requests.append((self.path, body))
        if self.path.endswith("pageInfo"):
            result = {"passwordEncrypt": "true", "publicKeyExponent": "01",
                      "publicKeyModulus": "FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF",
                      "validCodeUrl": "", "prefixName": "false"}
        else:
            result = {"result": "success", "userIndex": "test"}
        data = json.dumps(result).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def do_GET(self):
        if self.path == "/redirect":
            self.send_response(302)
            self.send_header("Location", f"http://127.0.0.1:{self.server.server_port}/eportal/index.jsp?mac=12&x=3")
        else:
            self.send_response(200)
        self.end_headers()

    def log_message(self, *args):
        pass


class AuthTests(unittest.TestCase):
    def test_network_check_has_total_deadline_even_if_open_ignores_timeout(self):
        release = Event()

        class StalledOpener:
            def open(self, request, timeout):
                release.wait(0.4)
                raise TimeoutError()

        try:
            with patch.object(hust_auth, "build_opener", return_value=StalledOpener()), \
                    patch.object(hust_auth, "PROBES", (("http://slow-a.invalid/", 204), ("http://slow-b.invalid/", 204))), \
                    patch.object(hust_auth, "NETWORK_CHECK_TIMEOUT", 0.12, create=True):
                start = time.monotonic()
                result = hust_auth.Portal("http://slow-campus.invalid/eportal/").check_network()
                elapsed = time.monotonic() - start
        finally:
            release.set()
        self.assertEqual(result, ("unreachable", ""))
        self.assertLess(elapsed, 0.3)

    def test_fast_domestic_probe_returns_without_waiting_for_stalled_probe(self):
        release = Event()

        class MixedOpener:
            def open(self, request, timeout):
                if request.full_url == "http://fast-domestic.invalid/":
                    response = io.BytesIO(b"")
                    response.status = 204
                    response.url = request.full_url
                    return response
                release.wait(0.4)
                raise TimeoutError()

        try:
            with patch.object(hust_auth, "build_opener", return_value=MixedOpener()), \
                    patch.object(hust_auth, "PROBES", (("http://blocked-first.invalid/", 204), ("http://fast-domestic.invalid/", 204))):
                start = time.monotonic()
                result = hust_auth.Portal("http://blocked-campus.invalid/eportal/").check_network()
                elapsed = time.monotonic() - start
        finally:
            release.set()
        self.assertEqual(result, ("online", ""))
        self.assertLess(elapsed, 0.3)

    def test_repeated_checks_do_not_spawn_more_workers_for_stalled_urls(self):
        release = Event()
        calls = []

        class StalledOpener:
            def open(self, request, timeout):
                calls.append(request.full_url)
                release.wait(0.4)
                raise TimeoutError()

        try:
            with patch.object(hust_auth, "build_opener", return_value=StalledOpener()), \
                    patch.object(hust_auth, "PROBES", (("http://persistent-slow.invalid/", 204),)), \
                    patch.object(hust_auth, "NETWORK_CHECK_TIMEOUT", 0.05, create=True):
                portal = hust_auth.Portal("http://persistent-campus.invalid/eportal/")
                portal.check_network()
                portal.check_network()
        finally:
            release.set()
        self.assertLessEqual(len(calls), 2)

    def test_no_command_opens_menu_and_zero_exits(self):
        output = io.StringIO()
        with patch("builtins.input", return_value="0"), redirect_stdout(output), redirect_stderr(io.StringIO()):
            try:
                result = hust_auth.main([])
            except SystemExit as exc:
                result = exc.code
        self.assertEqual(result, 0)
        self.assertIn("立即登录", output.getvalue())

    def test_menu_checks_status_then_returns_to_choices(self):
        output = io.StringIO()
        with patch("builtins.input", side_effect=["2", "0"]), \
                patch.object(hust_auth.Portal, "check_network", return_value=("online", "")) as check, \
                patch.object(hust_auth.time, "monotonic", side_effect=[10.0, 10.1234]), \
                redirect_stdout(output):
            self.assertEqual(hust_auth.main([]), 0)
        check.assert_called_once_with()
        self.assertIn("已经联网", output.getvalue())
        self.assertIn("延迟 123 ms", output.getvalue())
        self.assertNotIn("耗时", output.getvalue())

    def test_menu_displays_options_in_horizontal_rows(self):
        output = io.StringIO()
        with patch("builtins.input", return_value="0"), \
                redirect_stdout(output):
            self.assertEqual(hust_auth.main([]), 0)
        first_row = next(line for line in output.getvalue().splitlines() if "[1]" in line)
        self.assertIn("[2]", first_row)
        self.assertIn("[3]", first_row)

    def test_menu_login_runs_without_confirmation(self):
        output = io.StringIO()
        with patch("builtins.input", side_effect=["1", "0"]), \
                patch.object(hust_auth, "load_credentials", return_value=("student", "secret", "")), \
                patch.object(hust_auth.Portal, "check_network", return_value=("online", "")), \
                redirect_stdout(output):
            self.assertEqual(hust_auth.main([]), 0)
        self.assertIn("已经联网，无需认证", output.getvalue())
        self.assertNotIn("(y/n)", output.getvalue())

    def test_menu_disables_autostart_without_confirmation(self):
        with patch("builtins.input", side_effect=["6", "0"]), \
                patch.object(hust_auth, "uninstall_autostart") as uninstall, \
                redirect_stdout(io.StringIO()):
            self.assertEqual(hust_auth.main([]), 0)
        uninstall.assert_called_once_with()

    def test_autostart_uses_isolated_site_free_python(self):
        with patch.object(hust_auth.sys, "executable", r"C:\Python\python.exe"), \
                patch.object(hust_auth, "__file__", r"D:\Apps\hust_auth.py"):
            command = hust_auth.autostart_args()
        self.assertEqual(command[:3], [r"C:\Python\pythonw.exe", "-I", "-S"])
        self.assertEqual(command[-2:], [r"D:\Apps\hust_auth.py", "watch"])

    def test_menu_cancel_does_not_remove_tool_data(self):
        output = io.StringIO()
        with patch("builtins.input", side_effect=["8", "n", "0"]), \
                patch.object(hust_auth, "remove_tool_data") as remove, \
                redirect_stdout(output):
            self.assertEqual(hust_auth.main([]), 0)
        remove.assert_not_called()
        self.assertIn("已取消", output.getvalue())

    def test_menu_confirmed_remove_cleans_only_known_user_files(self):
        with tempfile.TemporaryDirectory() as folder:
            config = Path(folder) / "config" / "config.json"
            config.parent.mkdir()
            log = config.with_name("hust-auth.log")
            saved_url = Path(folder) / "wired-login-url.txt"
            unrelated = config.parent / "keep.txt"
            for path in (config, log, saved_url, unrelated):
                path.write_text("test", encoding="utf-8")
            with patch("builtins.input", side_effect=["8", "y", "0"]), \
                    patch.object(hust_auth, "config_path", return_value=config), \
                    patch.object(hust_auth, "login_url_path", return_value=saved_url), \
                    patch.object(hust_auth, "legacy_login_url_path", return_value=Path(folder) / "legacy.txt", create=True), \
                    patch.object(hust_auth, "uninstall_autostart") as uninstall, \
                    redirect_stdout(io.StringIO()):
                self.assertEqual(hust_auth.main([]), 0)
            uninstall.assert_called_once_with()
            self.assertFalse(config.exists())
            self.assertFalse(log.exists())
            self.assertFalse(saved_url.exists())
            self.assertTrue(unrelated.exists())

    def test_setup_only_saves_credentials_without_enabling_autostart(self):
        output = io.StringIO()
        with patch("builtins.input", side_effect=["student", ""]), \
                patch("getpass.getpass", return_value="secret"), \
                patch.object(hust_auth, "save_credentials") as save, \
                patch.object(hust_auth, "install_autostart") as install, \
                redirect_stdout(output):
            self.assertEqual(hust_auth.main(["setup"]), 0)
        save.assert_called_once_with("student", "secret", "")
        install.assert_not_called()
        self.assertIn("HTTP", output.getvalue())

    def test_menu_login_guides_url_paste_then_authenticates_once(self):
        url = hust_auth.PORTAL_URL + "index.jsp?wlanuserip=sample&nasip=sample&mac=sample"
        output = io.StringIO()
        with tempfile.TemporaryDirectory() as folder:
            config = Path(folder) / "config.json"
            with patch.object(hust_auth, "config_path", return_value=config), \
                    patch.object(hust_auth, "login_url_path", return_value=Path(folder) / "wired-login-url.txt"), \
                    patch.object(hust_auth, "legacy_login_url_path", return_value=Path(folder) / "legacy.txt", create=True), \
                    patch("builtins.input", side_effect=["1", "http://example.invalid/", url, "0"]), \
                    patch.object(hust_auth, "load_credentials", return_value=("student", "secret", "")), \
                    patch.object(hust_auth.Portal, "check_network", return_value=("missing_query", "")), \
                    patch.object(hust_auth.Portal, "login") as login, \
                    redirect_stdout(output), redirect_stderr(io.StringIO()):
                self.assertEqual(hust_auth.main([]), 0)
                saved = hust_auth.login_url_path()
                self.assertEqual(saved.parent, config.parent)
                self.assertEqual(saved.read_text(encoding="utf-8"), url)
            login.assert_called_once_with("student", "secret", "wlanuserip=sample&nasip=sample&mac=sample", "")
            self.assertNotIn(url, output.getvalue())

    def test_cancelled_url_guidance_never_submits_login(self):
        output = io.StringIO()
        with tempfile.TemporaryDirectory() as folder:
            with patch.object(hust_auth, "config_path", return_value=Path(folder) / "config.json"), \
                    patch.object(hust_auth, "login_url_path", return_value=Path(folder) / "wired-login-url.txt"), \
                    patch.object(hust_auth, "legacy_login_url_path", return_value=Path(folder) / "legacy.txt", create=True), \
                    patch("builtins.input", side_effect=["1", "", "0"]), \
                    patch.object(hust_auth, "load_credentials", return_value=("student", "secret", "")), \
                    patch.object(hust_auth.Portal, "check_network", return_value=("missing_query", "")), \
                    patch.object(hust_auth.Portal, "login") as login, \
                    redirect_stdout(output), redirect_stderr(io.StringIO()):
                self.assertEqual(hust_auth.main([]), 0)
            login.assert_not_called()
        self.assertIn("已取消", output.getvalue())

    def test_url_update_rejects_success_page_without_overwriting(self):
        with tempfile.TemporaryDirectory() as folder:
            saved = Path(folder) / "wired-login-url.txt"
            saved.write_text("existing", encoding="utf-8")
            output = io.StringIO()
            with patch.object(hust_auth, "login_url_path", return_value=saved), \
                    patch("builtins.input", side_effect=["9", hust_auth.PORTAL_URL + "success.jsp?userIndex=example", "", "0"]), \
                    redirect_stdout(output):
                self.assertEqual(hust_auth.main([]), 0)
            self.assertEqual(saved.read_text(encoding="utf-8"), "existing")
            self.assertIn("index.jsp", output.getvalue())

    def test_valid_legacy_url_is_migrated_to_user_config(self):
        url = hust_auth.PORTAL_URL + "index.jsp?wlanuserip=sample&nasip=sample"
        with tempfile.TemporaryDirectory() as folder:
            saved = Path(folder) / "new" / "wired-login-url.txt"
            legacy = Path(folder) / "legacy.txt"
            legacy.write_text(url, encoding="utf-8")
            with patch.object(hust_auth, "login_url_path", return_value=saved), \
                    patch.object(hust_auth, "legacy_login_url_path", return_value=legacy, create=True):
                self.assertEqual(hust_auth.load_login_query(hust_auth.PORTAL_URL), "wlanuserip=sample&nasip=sample")
            self.assertEqual(saved.read_text(encoding="utf-8"), url)

    def test_default_login_url_storage_is_in_user_config(self):
        with patch.object(hust_auth, "config_path", return_value=Path("user-data") / "config.json"):
            self.assertEqual(hust_auth.login_url_path(), Path("user-data") / "wired-login-url.txt")

    def test_detected_query_takes_priority_and_is_saved_after_success(self):
        class DetectedPortal:
            base_url = hust_auth.PORTAL_URL
            received = None

            def check_network(self):
                return "need_auth", "wlanuserip=fresh&nasip=fresh"

            def login(self, username, password, query, service):
                self.received = query

        portal = DetectedPortal()
        with tempfile.TemporaryDirectory() as folder:
            saved = Path(folder) / "wired-login-url.txt"
            saved.write_text("old", encoding="utf-8")
            with patch.object(hust_auth, "login_url_path", return_value=saved), \
                    patch("builtins.input", side_effect=AssertionError("不可询问输入")):
                hust_auth.login_once(portal, ("student", "secret", ""))
            self.assertEqual(portal.received, "wlanuserip=fresh&nasip=fresh")
            self.assertEqual(saved.read_text(encoding="utf-8"), portal.base_url + "index.jsp?wlanuserip=fresh&nasip=fresh")

    def test_successful_login_is_not_reported_as_failure_when_cache_is_unwritable(self):
        portal = hust_auth.Portal()
        with patch.object(portal, "check_network", return_value=("need_auth", "wlanuserip=sample&nasip=sample")), \
                patch.object(portal, "login") as login, \
                patch.object(hust_auth, "save_login_url", side_effect=OSError("unwritable")):
            message = hust_auth.login_once(portal, ("student", "secret", ""))
        login.assert_called_once()
        self.assertIn("已接受登录", message)
        self.assertIn("保存失败", message)

    def test_rsa_matches_portal_little_endian_blocks(self):
        self.assertEqual(hust_auth.rsa_encrypt("abc", "12", "01", "FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF"),
                         "6162633e3132")

    def test_login_refuses_plaintext_password_on_http(self):
        portal = hust_auth.Portal()
        with patch.object(portal, "_post", return_value={"passwordEncrypt": "false"}) as post:
            with self.assertRaisesRegex(hust_auth.PortalError, "明文"):
                portal.login("student", "secret", "nasip=sample&wlanuserip=sample")
        self.assertEqual(post.call_count, 1)

    def test_malformed_portal_public_key_is_reported(self):
        with self.assertRaises(hust_auth.PortalError):
            hust_auth.rsa_encrypt("secret", "mac", "", "bad-key")

    def test_login_sends_double_encoded_fields(self):
        PortalHandler.requests = []
        server = ThreadingHTTPServer(("127.0.0.1", 0), PortalHandler)
        thread = Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            portal = hust_auth.Portal(f"http://127.0.0.1:{server.server_port}/eportal/")
            result = portal.login("user@hust", "abc", "mac=12&x=a%20b", "")
            self.assertEqual(result["result"], "success")
            self.assertEqual(len(PortalHandler.requests), 2)
            page = parse_qs(PortalHandler.requests[0][1])
            login = parse_qs(PortalHandler.requests[1][1])
            self.assertEqual(page["queryString"], ["mac=12&x=a%20b"])
            self.assertEqual(login["userId"], ["user%40hust"])
            self.assertEqual(login["queryString"], ["mac%3D12%26x%3Da%2520b"])
            self.assertEqual(login["passwordEncrypt"], ["true"])
            self.assertEqual(login["password"], ["6162633e3132"])
        finally:
            server.shutdown()
            server.server_close()

    def test_captive_portal_redirect_supplies_query_string(self):
        server = ThreadingHTTPServer(("127.0.0.1", 0), PortalHandler)
        thread = Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            base = f"http://127.0.0.1:{server.server_port}"
            with patch.object(hust_auth, "PROBES", ((base + "/redirect", b"ok"),)):
                self.assertEqual(hust_auth.Portal(base + "/eportal/").check_network(),
                                 ("need_auth", "mac=12&x=3"))
        finally:
            server.shutdown()
            server.server_close()

    def test_missing_redirect_query_never_attempts_login(self):
        PortalHandler.requests = []
        server = ThreadingHTTPServer(("127.0.0.1", 0), PortalHandler)
        thread = Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            base = f"http://127.0.0.1:{server.server_port}"
            with patch.object(hust_auth, "PROBES", ()):
                portal = hust_auth.Portal(base + "/eportal/")
                self.assertEqual(portal.check_network(), ("missing_query", ""))
                with tempfile.TemporaryDirectory() as folder:
                    with patch.object(hust_auth, "login_url_path", return_value=Path(folder) / "missing.txt"), \
                            patch.object(hust_auth, "legacy_login_url_path", return_value=Path(folder) / "legacy.txt"), \
                            patch("builtins.input", side_effect=AssertionError("非交互登录不得询问输入")):
                        with self.assertRaisesRegex(hust_auth.PortalError, "认证参数"):
                            hust_auth.login_once(portal, ("student", "secret", ""))
            self.assertEqual(PortalHandler.requests, [])
        finally:
            server.shutdown()
            server.server_close()

    def test_watch_keeps_waiting_when_redirect_query_is_missing(self):
        class PortalWithoutRedirect:
            base_url = "http://172.18.18.61:8080/eportal/"

            def check_network(self):
                return "missing_query", ""

            def login(self, *args):
                raise AssertionError("login must not be called")

        class StopLoop(Exception):
            pass

        with tempfile.TemporaryDirectory() as folder:
            config = Path(folder) / "config.json"
            config.write_text("test", encoding="utf-8")
            with patch.object(hust_auth, "config_path", return_value=config), \
                    patch.object(hust_auth, "load_login_query", side_effect=hust_auth.LoginUrlRequired("未取得认证参数；请选择菜单 9")), \
                    patch("builtins.input", side_effect=AssertionError("后台不得询问输入")), \
                    patch.object(hust_auth.time, "sleep", side_effect=[None, None, None, None, StopLoop()]):
                with self.assertRaises(StopLoop):
                    hust_auth.watch(PortalWithoutRedirect(), ("student", "secret", ""))
            self.assertEqual(config.with_name("hust-auth.log").read_text(encoding="utf-8").count("菜单 9"), 1)

    def test_watch_stops_when_uninstall_removes_config(self):
        class OnlinePortal:
            calls = 0

            def check_network(self):
                self.calls += 1
                return "online", ""

        with tempfile.TemporaryDirectory() as folder:
            config = Path(folder) / "config.json"
            config.write_text("test", encoding="utf-8")
            portal = OnlinePortal()
            sleeps = 0

            def remove_config_on_first_sleep(_):
                nonlocal sleeps
                sleeps += 1
                if sleeps == 1:
                    config.unlink()
                else:
                    raise AssertionError("监测进程应在配置删除后退出")

            with patch.object(hust_auth, "config_path", return_value=config), \
                    patch.object(hust_auth.time, "sleep", side_effect=remove_config_on_first_sleep):
                self.assertIsNone(hust_auth.watch(portal, ("student", "secret", "")))
            self.assertEqual(portal.calls, 1)
            self.assertEqual(sleeps, 1)

    def test_saved_login_page_supplies_query_when_redirect_is_blocked(self):
        class PortalWithoutRedirect:
            base_url = "http://172.18.18.61:8080/eportal/"
            received_query = None

            def check_network(self):
                return "missing_query", ""

            def login(self, username, password, query, service):
                self.received_query = query

        portal = PortalWithoutRedirect()
        url = portal.base_url + "index.jsp?wlanuserip=10.17.1.2&nasip=10.1.1.1&mac=AABBCC"
        with tempfile.TemporaryDirectory() as folder:
            path = os.path.join(folder, "wired-login-url.txt")
            with open(path, "w", encoding="utf-8") as stream:
                stream.write(url)
            with patch.object(hust_auth, "login_url_path", return_value=Path(path)):
                self.assertEqual(hust_auth.login_once(portal, ("student", "secret", "")),
                                 "认证服务器已接受登录")
        self.assertEqual(portal.received_query,
                         "wlanuserip=10.17.1.2&nasip=10.1.1.1&mac=AABBCC")

    def test_saved_success_page_is_rejected_as_login_url(self):
        with self.assertRaisesRegex(hust_auth.PortalError, "index.jsp"):
            hust_auth.parse_login_url(
                "http://172.18.18.61:8080/eportal/success.jsp?userIndex=example",
                "http://172.18.18.61:8080/eportal/")

    def test_malformed_login_url_reports_validation_error(self):
        with self.assertRaises(hust_auth.PortalError):
            hust_auth.parse_login_url("http://[invalid/", hust_auth.PORTAL_URL)

    def test_encoded_login_parameters_are_preserved(self):
        query = "wlanuserip=" + "a" * 32 + "&nasip=" + "b" * 32
        self.assertEqual(hust_auth.parse_login_url(
            "http://172.18.18.61:8080/eportal/index.jsp?" + query,
            "http://172.18.18.61:8080/eportal/"), query)

    @unittest.skipUnless(os.name == "nt", "DPAPI is Windows only")
    def test_saved_password_is_protected_and_readable_by_current_user(self):
        with tempfile.TemporaryDirectory() as folder, patch.dict(os.environ, {"APPDATA": folder}):
            hust_auth.save_credentials("student", "private-password", "")
            saved = hust_auth.config_path().read_text(encoding="utf-8")
            self.assertNotIn("private-password", saved)
            self.assertEqual(hust_auth.load_credentials(), ("student", "private-password", ""))


if __name__ == "__main__":
    unittest.main()
