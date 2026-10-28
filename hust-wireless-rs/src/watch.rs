use std::fs;
use std::io::{self, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use chrono::Local;

use crate::config::{self, Credentials};
use crate::error::{AppError, Result};
use crate::portal::{self, Portal};

static STOP: AtomicBool = AtomicBool::new(false);

pub fn stop_requested() -> bool {
    STOP.load(Ordering::SeqCst)
}

pub fn request_stop() {
    STOP.store(true, Ordering::SeqCst);
    portal::request_cancel();
}

pub fn clear_stop() {
    STOP.store(false, Ordering::SeqCst);
    portal::clear_cancel();
}

/// Returns true when the user pressed Q or q.
pub fn quit_pressed() -> bool {
    match read_key() {
        Some(ch) if ch == b'q' as u16 || ch == b'Q' as u16 => {
            request_stop();
            true
        }
        _ => false,
    }
}

fn read_key() -> Option<u16> {
    #[cfg(windows)]
    {
        unsafe extern "C" {
            fn _kbhit() -> i32;
            fn _getwch() -> u16;
        }
        if unsafe { _kbhit() } != 0 {
            let ch = unsafe { _getwch() };
            if ch == 0 || ch == 0xE0 {
                if unsafe { _kbhit() } != 0 {
                    let _ = unsafe { _getwch() };
                }
                return None;
            }
            return Some(ch);
        }
    }
    read_piped_key()
}

fn read_piped_key() -> Option<u16> {
    use std::io::Read;
    #[cfg(unix)]
    use std::os::unix::io::AsRawFd;
    #[cfg(windows)]
    use std::os::windows::io::AsRawHandle;

    #[cfg(unix)]
    {
        let mut fds = [libc::pollfd {
            fd: io::stdin().as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        }];
        let ready = unsafe { libc::poll(fds.as_mut_ptr(), 1, 0) };
        if ready <= 0 {
            return None;
        }
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::Storage::FileSystem::GetFileType;
        let handle = io::stdin().as_raw_handle() as *mut _;
        let kind = unsafe { GetFileType(handle) };
        // FILE_TYPE_CHAR is a real console; _kbhit already covered it.
        if kind == 0x0002 {
            return None;
        }
        let mut avail = 0u32;
        let ok = unsafe {
            windows_sys::Win32::System::Pipes::PeekNamedPipe(
                handle,
                std::ptr::null_mut(),
                0,
                std::ptr::null_mut(),
                &mut avail,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 || avail == 0 {
            return None;
        }
    }
    let mut byte = [0u8; 1];
    match io::stdin().lock().read(&mut byte) {
        Ok(1) if byte[0] == b'\r' || byte[0] == b'\n' => None,
        Ok(1) => Some(byte[0] as u16),
        _ => None,
    }
}

pub fn append_log(message: &str, level: &str) {
    let path = config::log_path();
    if !path.parent().is_some_and(|parent| parent.exists()) {
        return;
    }
    let timestamp = Local::now().format("%Y-%m-%d %H:%M:%S,%3f");
    if let Ok(mut file) = fs::OpenOptions::new().create(true).append(true).open(path) {
        use std::io::Write;
        let _ = writeln!(file, "{timestamp} {level} {message}");
    }
}

fn say(message: &str) {
    println!("{message}");
    let _ = io::stdout().flush();
}

pub fn watch(portal: &Portal, credentials: &Credentials, quiet: bool) -> Result<()> {
    let path = config::config_path();
    if !path.exists() {
        return Ok(());
    }

    clear_stop();
    if !quiet {
        say("持续监测已开始，按 Q 返回菜单。");
    }

    let mut failures = 0u32;
    let mut url_notice_logged = false;
    while path.exists() {
        if stop_requested() || (!quiet && quit_pressed()) {
            if !quiet {
                say("已停止监测，返回菜单。");
            }
            clear_stop();
            return Ok(());
        }

        if !quiet {
            say("正在检测中…");
        }
        let outcome = crate::portal::login_once(portal, credentials);
        if stop_requested() {
            if !quiet {
                say("已停止监测，返回菜单。");
            }
            clear_stop();
            return Ok(());
        }

        let delay = match outcome {
            Ok(message) => {
                if message != "已经联网，无需认证" {
                    append_log(&message, "INFO");
                }
                if !quiet {
                    say(&format!("检测结果：{message}"));
                }
                failures = 0;
                url_notice_logged = false;
                20
            }
            Err(AppError::Cancelled) => {
                if !quiet {
                    say("已停止监测，返回菜单。");
                }
                clear_stop();
                return Ok(());
            }
            Err(AppError::LoginUrlRequired(message)) => {
                if !url_notice_logged {
                    append_log(&message, "WARNING");
                    url_notice_logged = true;
                }
                if !quiet {
                    say(&format!("检测结果：{message}"));
                }
                failures = 0;
                20
            }
            Err(error) => {
                let message = error.to_string();
                if message.contains("尚未连接校园有线网") || message.contains("未取得认证参数")
                {
                    if !quiet {
                        say(&format!("检测结果：{message}"));
                    }
                    failures = 0;
                    20
                } else {
                    failures += 1;
                    append_log(&message, "ERROR");
                    if !quiet {
                        say(&format!("检测结果：{message}（连续失败 {failures}/5）"));
                    }
                    if failures >= 5 {
                        let stopped = "连续认证失败 5 次，监测已停止；请检查账号后重新启动";
                        append_log(stopped, "ERROR");
                        if !quiet {
                            say(stopped);
                        }
                        clear_stop();
                        return Ok(());
                    }
                    (20 * 2u64.pow(failures.min(4))).min(300)
                }
            }
        };

        if !quiet {
            say(&format!("{delay} 秒后再次检测…"));
        }
        for _ in 0..delay * 5 {
            if stop_requested() || (!quiet && quit_pressed()) {
                if !quiet {
                    say("已停止监测，返回菜单。");
                }
                clear_stop();
                return Ok(());
            }
            if !path.exists() {
                clear_stop();
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(200));
        }
    }
    clear_stop();
    Ok(())
}
