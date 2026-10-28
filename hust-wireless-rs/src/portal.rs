use std::collections::HashSet;
use std::io::Read;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock, mpsc};
use std::time::{Duration, Instant};

use reqwest::StatusCode;
use reqwest::blocking::Client;
use reqwest::header::{CONTENT_TYPE, LOCATION, USER_AGENT};
use reqwest::redirect::Policy;
use serde_json::Value;
use url::Url;

use crate::encoding::{double_encode, encode_component};
use crate::error::{AppError, Result};
use crate::login_url;
use crate::rsa;

pub const PORTAL_URL: &str = "http://172.18.18.61:8080/eportal/";
pub const NETWORK_CHECK_TIMEOUT: Duration = Duration::from_secs(3);
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(2);

const DEFAULT_PROBES: &[&str] = &[
    "http://connect.rom.miui.com/generate_204",
    "http://connectivitycheck.platform.hicloud.com/generate_204",
    "http://123.123.123.123/",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkState {
    Online,
    NeedAuth,
    MissingQuery,
    Unreachable,
}

impl NetworkState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Online => "已经联网",
            Self::NeedAuth => "需要认证",
            Self::MissingQuery => "需要认证，但未取得认证参数",
            Self::Unreachable => "检测超时或网络不可达",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkCheck {
    pub state: NetworkState,
    pub query: String,
}

#[derive(Debug, Clone)]
struct ProbeResult {
    state: NetworkState,
    query: String,
}

pub struct Portal {
    pub base_url: String,
    client: Client,
}

static CANCEL_CHECK: AtomicBool = AtomicBool::new(false);

pub fn request_cancel() {
    CANCEL_CHECK.store(true, Ordering::SeqCst);
}

pub fn clear_cancel() {
    CANCEL_CHECK.store(false, Ordering::SeqCst);
}

pub fn cancel_requested() -> bool {
    CANCEL_CHECK.load(Ordering::SeqCst)
}

impl Portal {
    pub fn new(base_url: &str) -> Result<Self> {
        let parsed = Url::parse(base_url)?;
        if parsed.scheme() != "http" || parsed.host_str().is_none() {
            return Err(AppError::msg("认证服务器地址必须是 HTTP URL"));
        }
        let base_url = format!("{}/", base_url.trim_end_matches('/'));
        let client = Client::builder()
            .redirect(Policy::none())
            .timeout(PROBE_TIMEOUT)
            .no_proxy()
            .build()?;
        Ok(Self { base_url, client })
    }

    pub fn check_network(&self) -> NetworkCheck {
        let deadline = Instant::now() + NETWORK_CHECK_TIMEOUT;
        let (tx, rx) = mpsc::channel();
        let mut pending = 0usize;

        let mut targets: Vec<(String, Option<u16>, bool)> = DEFAULT_PROBES
            .iter()
            .map(|url| ((*url).to_string(), Some(204), false))
            .collect();
        targets.push((format!("{}index.jsp", self.base_url), None, true));

        for (url, expected, portal_page) in targets {
            let key = format!("{}\n{}", self.base_url, url);
            if !claim_probe(&key) {
                continue;
            }
            pending += 1;
            let tx = tx.clone();
            let client = self.client.clone();
            let base_url = self.base_url.clone();
            std::thread::spawn(move || {
                let result = probe_url(&client, &base_url, &url, expected, portal_page);
                let _ = tx.send(result);
                release_probe(&key);
            });
        }
        drop(tx);

        let mut fallback = NetworkCheck {
            state: NetworkState::Unreachable,
            query: String::new(),
        };
        while pending > 0 {
            if cancel_requested() || crate::watch::quit_pressed() {
                break;
            }
            let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
                break;
            };
            let slice = remaining.min(Duration::from_millis(200));
            match rx.recv_timeout(slice) {
                Ok(Some(result)) => {
                    pending -= 1;
                    match result.state {
                        NetworkState::Online | NetworkState::NeedAuth => {
                            return NetworkCheck {
                                state: result.state,
                                query: result.query,
                            };
                        }
                        NetworkState::MissingQuery => {
                            fallback = NetworkCheck {
                                state: result.state,
                                query: result.query,
                            };
                        }
                        NetworkState::Unreachable => {}
                    }
                }
                Ok(None) => pending -= 1,
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if Instant::now() >= deadline {
                        break;
                    }
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        fallback
    }

    pub fn login(
        &self,
        username: &str,
        password: &str,
        query_string: &str,
        service: &str,
    ) -> Result<Value> {
        if cancel_requested() {
            return Err(AppError::Cancelled);
        }
        let page = self.post(
            "pageInfo",
            &[("queryString", encode_component(query_string))],
        )?;

        if page
            .get("validCodeUrl")
            .and_then(Value::as_str)
            .is_some_and(|value| !value.is_empty())
        {
            return Err(AppError::msg("当前需要验证码，请用网页登录"));
        }

        let mut username = username.to_string();
        if page.get("prefixName").and_then(Value::as_str) == Some("true")
            && let Some(prefix) = page
                .get("prefixValue")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
        {
            username = format!("{prefix}{username}");
        }

        if page.get("passwordEncrypt").and_then(Value::as_str) != Some("true") {
            return Err(AppError::msg(
                "认证页未启用密码加密；为避免通过 HTTP 发送明文密码，已停止登录",
            ));
        }

        let mac = query_string
            .split('&')
            .find_map(|part| {
                let (key, value) = part.split_once('=')?;
                key.eq_ignore_ascii_case("mac").then_some(value)
            })
            .unwrap_or("111111111");
        let password = rsa::rsa_encrypt(
            password,
            mac,
            page.get("publicKeyExponent")
                .and_then(Value::as_str)
                .unwrap_or_default(),
            page.get("publicKeyModulus")
                .and_then(Value::as_str)
                .unwrap_or_default(),
        )?;

        let values = [
            ("userId", double_encode(&username)),
            ("password", double_encode(&password)),
            ("service", double_encode(service)),
            ("queryString", double_encode(query_string)),
            ("operatorPwd", String::new()),
            ("operatorUserId", String::new()),
            ("validcode", String::new()),
            ("passwordEncrypt", double_encode("true")),
        ];
        let result = self.post("login", &values)?;
        if result.get("result").and_then(Value::as_str) != Some("success") {
            let message = result
                .get("message")
                .and_then(Value::as_str)
                .or_else(|| result.get("result").and_then(Value::as_str))
                .unwrap_or("未知原因");
            return Err(AppError::msg(format!("认证失败：{message}")));
        }
        Ok(result)
    }

    fn post(&self, method: &str, values: &[(&str, String)]) -> Result<Value> {
        let body = values
            .iter()
            .map(|(key, value)| format!("{key}={value}"))
            .collect::<Vec<_>>()
            .join("&");
        let url = format!("{}InterFace.do?method={method}", self.base_url);
        let response = self
            .client
            .post(url)
            .timeout(Duration::from_secs(6))
            .header(
                CONTENT_TYPE,
                "application/x-www-form-urlencoded; charset=UTF-8",
            )
            .body(body)
            .send()
            .map_err(|error| AppError::msg(format!("无法连接认证服务器：{error}")))?;
        if response.status() != StatusCode::OK {
            return Err(AppError::msg(format!(
                "认证服务器返回 HTTP {}",
                response.status().as_u16()
            )));
        }
        response
            .json()
            .map_err(|_| AppError::msg("认证服务器返回了无法解析的数据"))
    }
}

fn probe_url(
    client: &Client,
    base_url: &str,
    url: &str,
    expected: Option<u16>,
    portal_page: bool,
) -> Option<ProbeResult> {
    let portal = Url::parse(base_url).ok()?;
    let response = client
        .get(url)
        .header(USER_AGENT, "HUST-Wired-Auth/1.0")
        .send()
        .ok()?;
    let status = response.status();

    if expected.is_some_and(|code| status.as_u16() == code) {
        return Some(ProbeResult {
            state: NetworkState::Online,
            query: String::new(),
        });
    }

    if status.is_redirection() {
        let location = response
            .headers()
            .get(LOCATION)
            .and_then(|value| value.to_str().ok())?;
        let target = response.url().join(location).ok()?;
        if same_host(&target, &portal) {
            return Some(ProbeResult {
                state: if target.query().unwrap_or_default().is_empty() {
                    NetworkState::MissingQuery
                } else {
                    NetworkState::NeedAuth
                },
                query: target.query().unwrap_or_default().to_string(),
            });
        }
        return None;
    }

    if portal_page && status == StatusCode::OK {
        let query = response.url().query().unwrap_or_default();
        return Some(ProbeResult {
            state: if query.is_empty() {
                NetworkState::MissingQuery
            } else {
                NetworkState::NeedAuth
            },
            query: query.to_string(),
        });
    }

    let response_url = response.url().clone();
    let mut body = Vec::new();
    response.take(2048).read_to_end(&mut body).ok()?;
    if body
        .windows(7)
        .any(|window| window.eq_ignore_ascii_case(b"eportal"))
        && !response_url.query().unwrap_or_default().is_empty()
        && same_host(&response_url, &portal)
    {
        return Some(ProbeResult {
            state: NetworkState::NeedAuth,
            query: response_url.query().unwrap_or_default().to_string(),
        });
    }
    None
}

fn same_host(left: &Url, right: &Url) -> bool {
    left.host_str()
        .zip(right.host_str())
        .is_some_and(|(a, b)| a.eq_ignore_ascii_case(b))
}

fn active_probes() -> &'static Mutex<HashSet<String>> {
    static ACTIVE: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    ACTIVE.get_or_init(|| Mutex::new(HashSet::new()))
}

fn claim_probe(key: &str) -> bool {
    active_probes()
        .lock()
        .map(|mut active| active.insert(key.to_string()))
        .unwrap_or(false)
}

fn release_probe(key: &str) {
    if let Ok(mut active) = active_probes().lock() {
        active.remove(key);
    }
}

pub fn login_once(portal: &Portal, credentials: &crate::config::Credentials) -> Result<String> {
    let check = portal.check_network();
    if cancel_requested() {
        return Err(AppError::Cancelled);
    }
    match check.state {
        NetworkState::Online => return Ok("已经联网，无需认证".to_string()),
        NetworkState::Unreachable => {
            return Err(AppError::msg("尚未连接校园有线网，或认证服务器不可达"));
        }
        NetworkState::MissingQuery => {
            let query = login_url::load_login_query(&portal.base_url)?;
            portal.login(
                &credentials.username,
                &credentials.password,
                &query,
                &credentials.service,
            )?;
        }
        NetworkState::NeedAuth => {
            portal.login(
                &credentials.username,
                &credentials.password,
                &check.query,
                &credentials.service,
            )?;
            let url = format!("{}index.jsp?{}", portal.base_url, check.query);
            if login_url::parse_login_url(&url, &portal.base_url).is_ok()
                && login_url::save_login_url(&url, &portal.base_url).is_err()
            {
                return Ok(
                    "认证服务器已接受登录，但本地地址保存失败；可通过菜单 9 重新保存".to_string(),
                );
            }
        }
    }
    Ok("认证服务器已接受登录".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    type RecordedRequests = Arc<std::sync::Mutex<Vec<(String, String)>>>;

    #[test]
    fn local_redirect_supplies_query_string() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buffer = [0u8; 1024];
            let _ = stream.read(&mut buffer);
            let response = concat!(
                "HTTP/1.1 302 Found\r\n",
                "Location: /eportal/index.jsp?mac=12&x=3\r\n",
                "Content-Length: 0\r\n\r\n"
            );
            stream.write_all(response.as_bytes()).unwrap();
        });

        let portal = Portal::new(&format!("http://127.0.0.1:{}/eportal/", address.port())).unwrap();
        let probe = probe_url(
            &portal.client,
            &portal.base_url,
            &format!("http://127.0.0.1:{}/redirect", address.port()),
            None,
            false,
        )
        .unwrap();
        server.join().unwrap();
        assert_eq!(probe.state, NetworkState::NeedAuth);
        assert_eq!(probe.query, "mac=12&x=3");
    }

    #[test]
    fn login_sends_double_encoded_fields() {
        let (base, requests, stop) = start_portal_server();
        let portal = Portal::new(&format!("{base}/eportal/")).unwrap();

        let result = portal
            .login("user@hust", "abc", "mac=12&x=a%20b", "")
            .unwrap();
        assert_eq!(
            result.get("result").and_then(Value::as_str),
            Some("success")
        );

        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline && requests.lock().unwrap().len() < 2 {
            std::thread::sleep(Duration::from_millis(10));
        }
        stop.store(true, Ordering::SeqCst);

        let requests = requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        let page_body = &requests
            .iter()
            .find(|(path, _)| path.contains("method=pageInfo"))
            .unwrap()
            .1;
        let login_body = &requests
            .iter()
            .find(|(path, _)| path.contains("method=login"))
            .unwrap()
            .1;

        assert!(page_body.contains("queryString=mac%3D12%26x%3Da%2520b"));
        assert!(login_body.contains("userId=user%2540hust"));
        assert!(login_body.contains("queryString=mac%253D12%2526x%253Da%252520b"));
        let expected_password =
            double_encode(&rsa::rsa_encrypt("abc", "12", "01", &"F".repeat(32)).unwrap());
        assert!(login_body.contains(&format!("password={expected_password}")));
        assert!(login_body.contains("service=&"));
        assert!(login_body.contains("passwordEncrypt=true"));
    }

    enum ReadOutcome {
        Request(String, String),
        Timeout,
        Closed,
    }

    fn start_portal_server() -> (String, RecordedRequests, Arc<AtomicBool>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let requests = Arc::new(std::sync::Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let server_requests = Arc::clone(&requests);
        let server_stop = Arc::clone(&stop);

        std::thread::spawn(move || {
            while !server_stop.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let requests = Arc::clone(&server_requests);
                        let stop = Arc::clone(&server_stop);
                        std::thread::spawn(move || handle_connection(stream, requests, stop));
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    Err(_) => break,
                }
            }
        });

        (format!("http://{address}"), requests, stop)
    }

    fn handle_connection(mut stream: TcpStream, requests: RecordedRequests, stop: Arc<AtomicBool>) {
        let _ = stream.set_read_timeout(Some(Duration::from_millis(100)));
        while !stop.load(Ordering::SeqCst) {
            match read_request(&mut stream) {
                ReadOutcome::Request(headers, body) => {
                    let path = headers.lines().next().unwrap_or_default().to_string();
                    let is_page_info = path.contains("method=pageInfo");
                    requests.lock().unwrap().push((path, body));
                    let json = if is_page_info {
                        format!(
                            r#"{{"passwordEncrypt":"true","publicKeyExponent":"01","publicKeyModulus":"{}","validCodeUrl":"","prefixName":"false"}}"#,
                            "F".repeat(32)
                        )
                    } else {
                        r#"{"result":"success","userIndex":"test"}"#.to_string()
                    };
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: keep-alive\r\n\r\n{}",
                        json.len(),
                        json
                    );
                    if stream.write_all(response.as_bytes()).is_err() {
                        break;
                    }
                }
                ReadOutcome::Timeout => continue,
                ReadOutcome::Closed => break,
            }
        }
    }

    fn read_request(stream: &mut TcpStream) -> ReadOutcome {
        let mut buffer = Vec::new();
        let mut chunk = [0u8; 1024];
        loop {
            let read = match stream.read(&mut chunk) {
                Ok(0) => return ReadOutcome::Closed,
                Ok(read) => read,
                Err(error)
                    if error.kind() == std::io::ErrorKind::WouldBlock
                        || error.kind() == std::io::ErrorKind::TimedOut =>
                {
                    return ReadOutcome::Timeout;
                }
                Err(_) => return ReadOutcome::Closed,
            };
            buffer.extend_from_slice(&chunk[..read]);
            let Some(header_end) = buffer.windows(4).position(|window| window == b"\r\n\r\n")
            else {
                continue;
            };
            let headers = String::from_utf8_lossy(&buffer[..header_end]).to_string();
            let content_length = headers
                .lines()
                .find_map(|line| {
                    let (key, value) = line.split_once(':')?;
                    key.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().ok())
                        .flatten()
                })
                .unwrap_or(0);
            let needed = header_end + 4 + content_length;
            while buffer.len() < needed {
                let read = match stream.read(&mut chunk) {
                    Ok(0) => return ReadOutcome::Closed,
                    Ok(read) => read,
                    Err(_) => return ReadOutcome::Closed,
                };
                buffer.extend_from_slice(&chunk[..read]);
            }
            let body = String::from_utf8_lossy(&buffer[header_end + 4..needed]).to_string();
            return ReadOutcome::Request(headers, body);
        }
    }
}
