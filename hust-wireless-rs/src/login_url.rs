use std::fs;

use url::Url;

use crate::config;
use crate::error::{AppError, Result};

pub fn parse_login_url(value: &str, portal_base: &str) -> Result<String> {
    let page = Url::parse(value.trim())
        .map_err(|_| AppError::msg("网址格式无效，请重新复制浏览器地址栏的完整地址"))?;
    let portal =
        Url::parse(portal_base).map_err(|_| AppError::msg("认证服务器地址必须是 HTTP URL"))?;

    let expected_path = format!("{}/index.jsp", portal.path().trim_end_matches('/'));
    let same_origin = page.scheme() == portal.scheme()
        && page.host_str().map(str::to_ascii_lowercase)
            == portal.host_str().map(str::to_ascii_lowercase)
        && page.port_or_known_default() == portal.port_or_known_default();
    if !same_origin || page.path() != expected_path {
        return Err(AppError::msg(
            "保存的地址不是 index.jsp 登录页；请复制绿色登录按钮出现时的地址",
        ));
    }

    let query = page.query().unwrap_or_default();
    let mut has_nasip = false;
    let mut has_wlanuserip = false;
    for (key, value) in url::form_urlencoded::parse(query.as_bytes()) {
        if value.is_empty() {
            continue;
        }
        match key.as_ref() {
            "nasip" => has_nasip = true,
            "wlanuserip" => has_wlanuserip = true,
            _ => {}
        }
    }
    if !has_nasip || !has_wlanuserip {
        return Err(AppError::msg("登录页地址缺少 nasip 或 wlanuserip 认证参数"));
    }

    Ok(query.to_string())
}

pub fn save_login_url(value: &str, portal_base: &str) -> Result<String> {
    let value = value.trim();
    let query = parse_login_url(value, portal_base)?;
    let path = config::login_url_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, value)?;
    Ok(query)
}

pub fn load_login_query(portal_base: &str) -> Result<String> {
    let primary = config::login_url_path();
    let mut candidates = vec![primary.clone()];
    candidates.extend(config::legacy_login_url_paths());

    for path in candidates {
        let raw = match fs::read_to_string(&path) {
            Ok(raw) => raw,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        };
        let value = raw.trim_start_matches('\u{feff}').trim();
        let query = parse_login_url(value, portal_base).map_err(|_| {
            AppError::LoginUrlRequired(
                "未取得认证参数：本地登录地址无效；请选择菜单 9 更新登录地址".to_string(),
            )
        })?;
        if path != primary {
            let _ = save_login_url(value, portal_base);
        }
        return Ok(query);
    }

    Err(AppError::LoginUrlRequired(
        "未取得认证参数；请打开菜单 9 粘贴自己的 index.jsp 登录页地址".to_string(),
    ))
}

pub fn query_field_names(query: &str) -> Vec<String> {
    let mut names: Vec<String> = url::form_urlencoded::parse(query.as_bytes())
        .filter(|(_, value)| !value.is_empty())
        .map(|(key, _)| key.into_owned())
        .collect();
    names.sort();
    names.dedup();
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    const PORTAL: &str = "http://172.18.18.61:8080/eportal/";

    #[test]
    fn accepts_index_page_with_required_fields() {
        let url = format!("{PORTAL}index.jsp?wlanuserip=10.17.1.2&nasip=10.1.1.1&mac=AABBCC");
        assert_eq!(
            parse_login_url(&url, PORTAL).unwrap(),
            "wlanuserip=10.17.1.2&nasip=10.1.1.1&mac=AABBCC"
        );
    }

    #[test]
    fn rejects_success_page() {
        let url = format!("{PORTAL}success.jsp?userIndex=example");
        assert!(parse_login_url(&url, PORTAL).is_err());
    }

    #[test]
    fn rejects_missing_required_fields() {
        let url = format!("{PORTAL}index.jsp?nasip=sample");
        assert!(parse_login_url(&url, PORTAL).is_err());
    }

    #[test]
    fn rejects_blank_required_fields() {
        let url = format!("{PORTAL}index.jsp?nasip=&wlanuserip=10.0.0.1");
        assert!(parse_login_url(&url, PORTAL).is_err());
    }

    #[test]
    fn rejects_malformed_url() {
        assert!(parse_login_url("http://[invalid/", PORTAL).is_err());
    }

    #[test]
    fn preserves_encoded_query() {
        let query = format!("wlanuserip={}&nasip={}", "a".repeat(32), "b".repeat(32));
        let url = format!("{PORTAL}index.jsp?{query}");
        assert_eq!(parse_login_url(&url, PORTAL).unwrap(), query);
    }

    #[test]
    fn diagnostic_fields_skip_blank_values() {
        assert_eq!(
            query_field_names("nasip=&wlanuserip=10.0.0.1&mac=AABBCC"),
            vec!["mac", "wlanuserip"]
        );
    }
}
