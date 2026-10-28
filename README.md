# 华科有线网自动认证

适用于锐捷 ePortal（`172.18.18.61:8080`）的华科校园**有线网**命令行工具。它检测当前是否需要认证，只有遇到认证页时才提交登录。

这是非官方工具。请使用自己的校园网账号，并在需要时保存自己设备当前的 `index.jsp` 登录页地址。不要使用别人的登录地址，也不要把账号、密码或完整登录地址发到 Issue。

当前发布版本是 [hust-wireless-rs](hust-wireless-rs/)，用 Rust 写成，不依赖本机 Python。仓库里仍保留原来的 [hust_auth.py](hust_auth.py)，作为行为对照。

## 构建

需要 [Rust](https://rustup.rs/) 稳定版。在仓库根目录执行：

```powershell
cd .\hust-wireless-rs
cargo build --release
```

没有 Visual Studio C++ 链接器时，使用已安装的 64 位 MinGW：

```powershell
cargo +stable-x86_64-pc-windows-gnu build --release
```

GNU 目标要求 64 位 MinGW 的 `bin` 目录在 `PATH` 中。

构建完成后可以双击 [hust-wireless-rs/hust.cmd](hust-wireless-rs/hust.cmd)，或直接运行：

```powershell
.\hust-wireless-rs\target\release\hust-wireless-rs.exe
```

## 菜单

不带参数启动后进入菜单。输入数字执行，终端较窄时每行的选项会自动减少。

| 数字 | 作用 |
| --- | --- |
| 1 | 立即登录 |
| 2 | 检查网络 |
| 3 | 参数诊断（只显示字段名） |
| 4 | 设置账号 |
| 5 | 启用开机自启 |
| 6 | 取消开机自启 |
| 7 | 在当前窗口持续监测 |
| 8 | 卸载工具（需要 `y/n` 确认） |
| 9 | 更新登录地址 |
| 0 | 退出 |

按 `2` 或 `7` 后会立刻显示「正在检测中」。探测并行进行，单次等待最长约 3 秒，结果带本次延迟（毫秒）。检测超时只表示这次没能确认状态。

持续监测时按 **Q** 返回菜单。等待下一次检测时按 Q 会马上返回；正在检测时，最迟约 3 秒内返回。菜单上的其他操作仍用回车确认。直接运行 `watch` 子命令时同样按 Q 退出。

## 第一次使用

1. 选择 `4`，输入校园网账号和密码。运营商代码通常直接回车。
2. 选择 `1` 登录。工具会先自动获取认证参数。
3. 若自动获取失败，选择 `9`，粘贴自己浏览器里**绿色「连接 Login」按钮那一页**的完整 `index.jsp?` 地址。`success.jsp?userIndex=...` 登录成功页会被拒绝。直接回车可取消，错误网址不会覆盖已有配置。
4. 需要开机自动认证时选择 `5`。下次登录 Windows 后，后台大约每 20 秒检查一次；还没插网线时会继续等待。

连续 5 次认证失败后，监测会停止，避免反复提交错误密码。账号变更后重新选择 `4`，这不会改开机自启。认证页要求验证码、设备未注册，或登录参数过期时，需要手动处理后再登录。

如果以前能用、后来认证失败，先选择 `3` 看字段名。正常跳转应有 `nasip`、`wlanuserip` 等字段。字段还在但登录失败时，选择 `9` 更新登录地址。诊断不打印地址内容、账号或密码。

## 命令

```powershell
.\hust-wireless-rs\target\release\hust-wireless-rs.exe status
.\hust-wireless-rs\target\release\hust-wireless-rs.exe diagnose
.\hust-wireless-rs\target\release\hust-wireless-rs.exe login
.\hust-wireless-rs\target\release\hust-wireless-rs.exe watch
.\hust-wireless-rs\target\release\hust-wireless-rs.exe install
.\hust-wireless-rs\target\release\hust-wireless-rs.exe uninstall
.\hust-wireless-rs\target\release\hust-wireless-rs.exe set-url
.\hust-wireless-rs\target\release\hust-wireless-rs.exe remove
```

`uninstall` 只取消开机自启。`remove` 在确认后清除自启、账号、日志和本地登录地址，不删除程序文件。正在运行的后台监测会在配置被删掉后退出。

## 数据目录

```text
%APPDATA%\hust-wireless-cli\
├── config.json          # 账号；密码经 Windows DPAPI 保护
├── wired-login-url.txt  # 本机登录地址，明文
└── hust-auth.log        # 监测日志
```

程序不读取浏览器里保存的密码。旧位置的 `wired-login-url.txt` 可以读取，校验通过后会复制到上面的目录，原文件保持不动。

## 安全与限制

- 学校认证入口是 HTTP。RSA 公钥也从 HTTP 获取，不能防止中间人替换公钥，不能把它当成 HTTPS。
- DPAPI 只保护磁盘上的密码。同一 Windows 用户下运行的程序仍可能解密。登录地址文件含设备和会话参数。
- 连通性探测使用小米、华为的 HTTP `generate_204`，并同时探测校园认证页和 `123.123.123.123`。不使用 Microsoft、Apple 或 `1.1.1.1` 作为默认探测。
- 开机自启目前只在 Windows 上可用。
- 自动化测试使用本地模拟接口。真实断线登录仍要在校园有线网处于待认证状态时确认。

## 仓库内容

```text
hust-wireless-rs/   Rust 命令行工具
hust_auth.py        原来的 Python 实现
test_hust_auth.py   Python 测试
```

以下内容留在本机，不进入仓库：编译产物、`reference/` 里保存的认证页、`wired-login-url.txt`，以及尚未完成的托盘原型。

## 测试

```powershell
cd .\hust-wireless-rs
cargo test
```

Python 对照实现：

```powershell
python -m unittest -v test_hust_auth.py
```

测试不会提交真实账号或密码。

## 许可证

[MIT](LICENSE)
