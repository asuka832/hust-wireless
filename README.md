<h1 align="center">华科有线网</h1>

<p align="center">
插上网线，自动完成校园有线网认证。
</p>

<p align="center">
  <a href="https://github.com/asuka832/hust-wireless/releases"><img alt="release" src="https://img.shields.io/github/v/release/asuka832/hust-wireless?style=flat-square&color=3b82f6"></a>
  <img alt="Windows" src="https://img.shields.io/badge/Windows-x64-0078D4?style=flat-square&logo=windows&logoColor=white">
  <img alt="Rust" src="https://img.shields.io/badge/Rust-1.96+-DEA584?style=flat-square&logo=rust&logoColor=white">
  <a href="LICENSE"><img alt="license" src="https://img.shields.io/badge/license-MIT-22c55e?style=flat-square"></a>
  <img alt="HUST" src="https://img.shields.io/badge/%E5%8D%8E%E4%B8%AD%E7%A7%91%E6%8A%80%E5%A4%A7%E5%AD%A6-%E6%9C%89%E7%BA%BF%E7%BD%91-c41e3a?style=flat-square">
</p>

<p align="center">
  <b>简体中文</b> · <a href="README_EN.md">English</a>
</p>

---

Windows 命令行工具，面向使用锐捷 ePortal 的华科校园有线网。它先判断现在要不要认证，只有停在认证页时才提交登录。

> 非官方工具。请使用自己的校园网账号。不要把账号、密码或完整登录地址发到 Issue。

## 特性

- 下载即可运行，不必安装 Rust
- 数字菜单，窄窗口会自动收成更少的列
- 持续监测，掉线后自动重连，按 `Q` 回到菜单
- 登录 Windows 后可在后台继续检查
- 密码经 Windows DPAPI 保存在本机，程序不读取浏览器密码

## 下载使用

到 [Releases](https://github.com/asuka832/hust-wireless/releases/latest) 下载 `hust-wireless-rs-windows-x64.zip`，解压后双击 `hust.cmd`。

1. 选 `4`，输入校园网账号和密码。运营商代码一般直接回车。
2. 选 `1` 登录。程序会自己获取认证参数。
3. 如果取不到参数，选 `9`，粘贴浏览器里绿色「连接 Login」那一页的完整 `index.jsp?` 地址。登录成功页 `success.jsp?userIndex=...` 不会被接受。直接回车可以取消，错误地址不会覆盖原来的配置。
4. 想开机就认证，再选 `5`。

## 菜单

| 按键 | 作用 |
| --- | --- |
| `1` | 立即登录 |
| `2` | 检查网络 |
| `3` | 参数诊断，只显示字段名 |
| `4` | 设置账号 |
| `5` | 启用开机自启 |
| `6` | 取消开机自启 |
| `7` | 在当前窗口持续监测 |
| `8` | 卸载本机数据，需要输入 `y` 确认 |
| `9` | 更新登录地址 |
| `0` | 退出 |

按 `2` 或 `7` 后，终端会马上显示「正在检测中」。一次探测最长约 3 秒，结果里带这次的延迟。持续监测时按 **Q** 返回菜单；直接运行 `watch` 时，按 Q 退出程序。

也可以不进菜单：

```powershell
.\hust-wireless-rs.exe login
.\hust-wireless-rs.exe status
.\hust-wireless-rs.exe watch
.\hust-wireless-rs.exe install
.\hust-wireless-rs.exe uninstall
.\hust-wireless-rs.exe set-url
.\hust-wireless-rs.exe remove
```

`uninstall` 只取消开机自启。`remove` 会清掉自启、账号、日志和登录地址，不删除程序本身。

## 登录失败时

先选 `3`。正常的认证跳转会带有 `nasip`、`wlanuserip` 等字段，诊断只打印字段名。

以前能用、后来失败，多半是地址变了，选 `9` 重新粘贴当前的 `index.jsp?` 地址。认证页要求验证码，或提示设备未注册时，需要先在网页上处理，再回来登录。连续失败 5 次后，监测会自己停下。

## 本机数据

```text
%APPDATA%\hust-wireless-cli\
├── config.json          # 账号，密码为 DPAPI 密文
├── wired-login-url.txt  # 登录地址，明文
└── hust-auth.log        # 监测日志
```

## 从源码构建

需要 Rust 稳定版。

```powershell
cd .\hust-wireless-rs
cargo build --release
```

没有 Visual Studio 的 C++ 链接器时：

```powershell
cargo +stable-x86_64-pc-windows-gnu build --release
```

GNU 目标需要把 64 位 MinGW 的 `bin` 加进 `PATH`。编完后运行 `target\release\hust-wireless-rs.exe`，或双击 `hust.cmd`。

```powershell
cargo test
```

## 说明

- 学校认证入口是 HTTP，RSA 公钥也从这条链路获取，不能当成 HTTPS。
- DPAPI 只保护磁盘上的密码。同一 Windows 用户下的程序仍可能解密。登录地址里有设备和会话参数。
- 连通性探测使用小米、华为的 `generate_204`，并同时看校园认证页和 `123.123.123.123`。
- 开机自启只支持 Windows。

## 许可证

[MIT](LICENSE)
