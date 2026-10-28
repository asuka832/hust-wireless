# HUST 有线网认证（Rust）

华科校园有线网命令行认证工具。完整说明见仓库根目录的 README。

## 构建与运行

```powershell
cargo build --release
.\target\release\hust-wireless-rs.exe
```

没有 MSVC 链接器时：

```powershell
cargo +stable-x86_64-pc-windows-gnu build --release
```

也可以双击本目录的 `hust.cmd`。

## 行为

- 菜单数字：`1` 登录、`2` 检查网络、`3` 参数诊断、`4` 设置账号、`5` 开机自启、`6` 取消自启、`7` 持续监测、`8` 卸载、`9` 更新登录地址、`0` 退出。
- 按 `2` 或 `7` 会立刻显示「正在检测中」。探测并行执行，单次最长约 3 秒。
- 持续监测或检查网络时按 **Q** 返回。直接运行 `watch` 时按 Q 退出进程。
- 子命令：`setup`、`login`、`status`、`diagnose`、`watch`、`install`、`uninstall`、`remove`、`set-url`。
- 密码用 Windows DPAPI 保存到 `%APPDATA%\hust-wireless-cli\config.json`。
- 登录请求使用与认证页一致的 UTF-16 小端分块 RSA，字段做两次 URL 编码。
- 连续认证失败会退避；满 5 次后停止监测。
- `remove` 只清理当前用户配置目录，不改其他位置已有的 `wired-login-url.txt`。读取旧地址时只复制。

## 测试

```powershell
cargo test
```

覆盖 RSA 分块、编码、登录地址校验、菜单布局和本地模拟认证接口。
