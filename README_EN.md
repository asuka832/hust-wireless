<h1 align="center">HUST Wired</h1>

<p align="center">
Plug in the cable. Sign in to the campus wired network.
</p>

<p align="center">
  <a href="https://github.com/asuka832/hust-wireless/releases"><img alt="release" src="https://img.shields.io/github/v/release/asuka832/hust-wireless?style=flat-square&color=3b82f6"></a>
  <img alt="Windows" src="https://img.shields.io/badge/Windows-x64-0078D4?style=flat-square&logo=windows&logoColor=white">
  <img alt="Rust" src="https://img.shields.io/badge/Rust-1.96+-DEA584?style=flat-square&logo=rust&logoColor=white">
  <a href="LICENSE"><img alt="license" src="https://img.shields.io/badge/license-MIT-22c55e?style=flat-square"></a>
  <img alt="HUST" src="https://img.shields.io/badge/HUST-wired%20network-c41e3a?style=flat-square">
</p>

<p align="center">
  <a href="README.md">简体中文</a> · <b>English</b>
</p>

---

A Windows command-line tool for the HUST campus wired network on Ruijie ePortal. It checks whether authentication is required, and submits a login only when the portal page is waiting.

> Unofficial. Use your own campus account. Do not post your account, password, or full login URL in an issue.

## Features

- Run it from the release archive. Rust is not required.
- A numbered menu that wraps when the terminal is narrow.
- A watch loop that signs in again after a drop. Press `Q` to leave it.
- Optional sign-in check after Windows starts.
- The password stays on this PC, protected with Windows DPAPI. The program does not read browser passwords.

## Download

Get `hust-wireless-rs-windows-x64.zip` from [Releases](https://github.com/asuka832/hust-wireless/releases/latest) and double-click `hust.cmd`.

1. Choose `4` and enter your campus account and password. The operator code is usually left empty.
2. Choose `1` to sign in. The tool fetches the portal parameters itself.
3. If that fails, choose `9` and paste the full `index.jsp?` URL from the page that shows the green Login button. A success page (`success.jsp?userIndex=...`) is rejected. Press Enter to cancel. A bad URL does not replace a saved one.
4. Choose `5` if it should run after you sign in to Windows.

## Menu

| Key | Action |
| --- | --- |
| `1` | Sign in now |
| `2` | Check the network |
| `3` | Show portal field names only |
| `4` | Save the account |
| `5` | Start at Windows sign-in |
| `6` | Stop starting with Windows |
| `7` | Watch in this window |
| `8` | Remove local data, after a `y` confirmation |
| `9` | Update the login URL |
| `0` | Exit |

`2` and `7` print “正在检测中” immediately. One check waits at most about 3 seconds and reports the latency. During a watch, press **Q** to return to the menu. A standalone `watch` command exits on Q.

```powershell
.\hust-wireless-rs.exe login
.\hust-wireless-rs.exe status
.\hust-wireless-rs.exe watch
.\hust-wireless-rs.exe install
.\hust-wireless-rs.exe uninstall
.\hust-wireless-rs.exe set-url
.\hust-wireless-rs.exe remove
```

`uninstall` only removes the startup entry. `remove` clears startup, the account, the log, and the saved login URL. It leaves the program files in place.

## When login fails

Choose `3` first. A normal portal redirect carries fields such as `nasip` and `wlanuserip`. The diagnosis prints names, not values.

If it used to work and then stopped, the address has usually changed. Choose `9` and paste the current `index.jsp?` URL. A captcha or an unregistered-device message has to be handled in the browser before another login. The watch stops after five failed sign-ins in a row.

## Local data

```text
%APPDATA%\hust-wireless-cli\
├── config.json          # account, password stored with DPAPI
├── wired-login-url.txt  # login URL, plain text
└── hust-auth.log        # watch log
```

## Build from source

Rust stable is required.

```powershell
cd .\hust-wireless-rs
cargo build --release
```

Without the Visual Studio C++ linker:

```powershell
cargo +stable-x86_64-pc-windows-gnu build --release
```

The GNU target needs a 64-bit MinGW `bin` directory on `PATH`. Run `target\release\hust-wireless-rs.exe`, or double-click `hust.cmd`.

```powershell
cargo test
```

## Notes

- The campus portal is HTTP, and the RSA public key comes over the same path. That is not HTTPS.
- DPAPI protects the password on disk. Another program running as the same Windows user can still decrypt it. The login URL contains device and session parameters.
- Connectivity probes use Xiaomi and Huawei `generate_204` endpoints, plus the campus portal and `123.123.123.123`.
- Startup with Windows is available on Windows only.

## License

[MIT](LICENSE)
