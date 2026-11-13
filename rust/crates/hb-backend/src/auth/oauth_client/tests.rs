//! OAuthClient 的測試（用本機假伺服器，不連真的 Hayaku）。

use std::io::{Read, Write};
use std::path::Path;
use std::sync::{Arc, Mutex};

use chrono::{TimeDelta, Utc};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::*;
use crate::test_support::{MockServer, Recorded, Reply, form};

pub(crate) fn meta_reply(req: &Recorded) -> Reply {
    let base = format!("http://{}", req.headers.get("host").cloned().unwrap_or_default());
    Reply::json(
        200,
        json!({
            "authorization_endpoint": format!("{base}/authorize"),
            "token_endpoint": format!("{base}/token"),
            "registration_endpoint": format!("{base}/register"),
            "revocation_endpoint": format!("{base}/revoke"),
        })
        .to_string(),
    )
}

/// 存一份登入資料到暫存檔。
pub(crate) fn store_with(
    dir: &Path,
    access: Option<&str>,
    refresh: Option<&str>,
    expires_in_secs: i64,
) -> std::path::PathBuf {
    let path = dir.join("auth.bin");
    let data = AuthData {
        client_id: Some("cid".into()),
        redirect_uri: Some(format!("http://127.0.0.1:{CALLBACK_PORT}/callback")),
        access_token: access.map(str::to_owned),
        refresh_token: refresh.map(str::to_owned),
        expires_at: (Utc::now() + TimeDelta::seconds(expires_in_secs)).fixed_offset(),
    };
    token_store::save_to(&path, &data).unwrap();
    path
}

fn client(base: &str, store: &Path) -> OAuthClient {
    OAuthClient::with_store(crate::http::app_client(), base, store)
}

#[test]
fn 沒有登入資料_需要登入() {
    let dir = tempfile::tempdir().unwrap();
    let server = MockServer::start(|_| Reply::json(500, "{}"));
    let auth = client(&server.base_url, &dir.path().join("auth.bin"));
    assert!(!auth.has_session());
    assert_eq!(Err(Error::AuthRequired("需要登入 Hayaku".into())), rt::block_on(auth.get_access_token()));
    assert!(server.requests().is_empty());
}

#[test]
fn token_還沒過期_直接用不打伺服器() {
    let dir = tempfile::tempdir().unwrap();
    let server = MockServer::start(|_| Reply::json(500, "{}"));
    let store = store_with(dir.path(), Some("at-1"), Some("rt-1"), 3600);
    let auth = client(&format!("{}/", server.base_url), &store);
    assert!(auth.has_session());
    assert_eq!(format!("{}/mcp", server.base_url), auth.resource());
    assert_eq!(Ok("at-1".to_owned()), rt::block_on(auth.get_access_token()));
    assert!(server.requests().is_empty());
}

#[test]
fn 快過期_用refresh_token換新的並存檔() {
    let dir = tempfile::tempdir().unwrap();
    let server = MockServer::start(|req| match req.path.as_str() {
        "/.well-known/oauth-authorization-server" => meta_reply(req),
        "/token" => Reply::json(200, r#"{"access_token":"at-2","refresh_token":"rt-2","expires_in":7200}"#),
        _ => Reply::json(404, "{}"),
    });
    // 剩 30 秒（60 秒內）就要換
    let store = store_with(dir.path(), Some("at-1"), Some("rt-1"), 30);
    let auth = client(&server.base_url, &store);
    assert_eq!(Ok("at-2".to_owned()), rt::block_on(auth.get_access_token()));

    let token_req = server.requests().into_iter().find(|r| r.path == "/token").unwrap();
    assert_eq!("POST", token_req.method);
    let f = form(&token_req.body);
    assert_eq!("refresh_token", f["grant_type"]);
    assert_eq!("rt-1", f["refresh_token"]);
    assert_eq!("cid", f["client_id"]);
    assert_eq!(format!("{}/mcp", server.base_url), f["resource"]);

    let saved = token_store::load_from(&store);
    assert_eq!(Some("at-2"), saved.access_token.as_deref());
    assert_eq!(Some("rt-2"), saved.refresh_token.as_deref());
    let left = saved.expires_at.with_timezone(&Utc) - Utc::now();
    assert!(left > TimeDelta::seconds(7100) && left <= TimeDelta::seconds(7200));

    // 第二次直接用（中繼資料也有快取）
    let before = server.requests().len();
    assert_eq!(Ok("at-2".to_owned()), rt::block_on(auth.get_access_token()));
    assert_eq!(before, server.requests().len());
}

#[test]
fn refresh_被拒_清掉token_要重新登入() {
    let dir = tempfile::tempdir().unwrap();
    let server = MockServer::start(|req| match req.path.as_str() {
        "/.well-known/oauth-authorization-server" => meta_reply(req),
        _ => Reply::json(400, r#"{"error":"invalid_grant"}"#),
    });
    let store = store_with(dir.path(), Some("at-1"), Some("rt-1"), -10);
    let auth = client(&server.base_url, &store);
    assert_eq!(Err(Error::AuthRequired("需要登入 Hayaku".into())), rt::block_on(auth.get_access_token()));
    assert!(!auth.has_session());
    let saved = token_store::load_from(&store);
    assert_eq!((None, None), (saved.access_token, saved.refresh_token));
    assert_eq!(Some("cid"), saved.client_id.as_deref());
    assert_eq!(Err(Error::AuthRequired("登入已失效，請重新登入".into())), rt::block_on(auth.force_refresh()));
}

#[test]
fn refresh_伺服器錯誤_保留token() {
    let dir = tempfile::tempdir().unwrap();
    let server = MockServer::start(|req| match req.path.as_str() {
        "/.well-known/oauth-authorization-server" => meta_reply(req),
        _ => Reply::json(503, "busy"),
    });
    let store = store_with(dir.path(), Some("at-1"), Some("rt-1"), -10);
    let auth = client(&server.base_url, &store);
    assert_eq!(Err(Error::AuthRequired("登入已失效，請重新登入".into())), rt::block_on(auth.force_refresh()));
    // 503 不清 refresh token（下次還能試），只清 access token
    let saved = token_store::load_from(&store);
    assert_eq!(None, saved.access_token);
    assert_eq!(Some("rt-1"), saved.refresh_token.as_deref());
}

/// 模擬瀏覽器：照授權網址把 code 和 state 送回本機回呼，回傳回呼頁的內容。
fn browser(url: &str, port: u16, code: Option<&str>, error: Option<&str>, wrong_state: bool) -> String {
    let parsed = url::Url::parse(url).unwrap();
    let q: std::collections::HashMap<String, String> = parsed.query_pairs().into_owned().collect();
    let state = if wrong_state { "bad".to_owned() } else { q["state"].clone() };
    let mut target = format!("/callback?state={}", urlencoding::encode(&state));
    if let Some(code) = code {
        target.push_str(&format!("&code={code}"));
    }
    if let Some(error) = error {
        target.push_str(&format!("&error={error}&error_description=%E4%BD%BF%E7%94%A8%E8%80%85+%E6%8B%92%E7%B5%95"));
    }
    let mut s = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    s.write_all(format!("GET {target} HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n").as_bytes()).unwrap();
    let mut out = String::new();
    let _ = s.read_to_string(&mut out);
    out
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

#[test]
fn 登入_註冊_pkce_換token() {
    let dir = tempfile::tempdir().unwrap();
    let server = MockServer::start(|req| match req.path.as_str() {
        "/.well-known/oauth-authorization-server" => meta_reply(req),
        "/register" => Reply::json(201, r#"{"client_id":"new-client"}"#),
        "/token" => Reply::json(200, r#"{"access_token":"at-9","refresh_token":"rt-9","expires_in":"x"}"#),
        _ => Reply::json(404, "{}"),
    });
    let port = free_port();
    let opened: Arc<Mutex<Vec<String>>> = Arc::default();
    let pages: Arc<Mutex<Vec<String>>> = Arc::default();
    let threads: Arc<Mutex<Vec<std::thread::JoinHandle<()>>>> = Arc::default();
    let (o, p, t) = (opened.clone(), pages.clone(), threads.clone());
    let store = dir.path().join("auth.bin");
    let auth = OAuthClient::build(
        crate::http::app_client(),
        &server.base_url,
        &store,
        port,
        Box::new(move |url| {
            o.lock().unwrap().push(url.to_owned());
            let url = url.to_owned();
            let p = p.clone();
            t.lock().unwrap().push(std::thread::spawn(move || {
                // state 不符：不算數，繼續等
                p.lock().unwrap().push(browser(&url, port, Some("c-1"), None, true));
                p.lock().unwrap().push(browser(&url, port, Some("c-1"), None, false));
            }));
            Ok(())
        }),
    );
    rt::block_on(auth.login()).unwrap();
    for t in threads.lock().unwrap().drain(..) {
        t.join().unwrap();
    }

    let reqs = server.requests();
    let register = reqs.iter().find(|r| r.path == "/register").unwrap();
    let payload: Value = serde_json::from_str(&register.body).unwrap();
    let redirect = format!("http://127.0.0.1:{port}/callback");
    assert_eq!(
        json!({
            "client_name": "HayakuBar 工時小工具",
            "redirect_uris": [redirect],
            "grant_types": ["authorization_code", "refresh_token"],
            "response_types": ["code"],
            "token_endpoint_auth_method": "none",
            "application_type": "native",
            "scope": "hayaku:read hayaku:write",
        }),
        payload
    );

    let url = url::Url::parse(&opened.lock().unwrap()[0]).unwrap();
    assert_eq!(
        format!("{}/authorize", server.base_url),
        format!("{}{}", url.origin().ascii_serialization(), url.path())
    );
    let q: std::collections::HashMap<String, String> = url.query_pairs().into_owned().collect();
    assert_eq!("code", q["response_type"]);
    assert_eq!("new-client", q["client_id"]);
    assert_eq!(redirect, q["redirect_uri"]);
    assert_eq!("hayaku:read hayaku:write", q["scope"]);
    assert_eq!("S256", q["code_challenge_method"]);
    assert_eq!(format!("{}/mcp", server.base_url), q["resource"]);
    assert!(opened.lock().unwrap()[0].contains("scope=hayaku%3Aread%20hayaku%3Awrite"));

    let token = reqs.iter().find(|r| r.path == "/token").unwrap();
    let f = form(&token.body);
    assert_eq!("authorization_code", f["grant_type"]);
    assert_eq!("c-1", f["code"]);
    assert_eq!(redirect, f["redirect_uri"]);
    assert_eq!("new-client", f["client_id"]);
    // PKCE：challenge = base64url(SHA256(verifier))
    let challenge =
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(f["code_verifier"].as_bytes()));
    assert_eq!(challenge, q["code_challenge"]);
    assert_eq!(43, f["code_verifier"].len());

    let pages = pages.lock().unwrap();
    assert!(pages[0].contains("<h2>登入失敗</h2><p>state 不符，請回到 HayakuBar 重新登入。</p>"), "{}", pages[0]);
    assert!(pages[1].starts_with("HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8"));
    assert!(
        pages[1].contains("<title>登入完成</title>") && pages[1].contains("<p>可以關閉這個分頁，回到 HayakuBar。</p>")
    );

    let saved = token_store::load_from(&store);
    assert_eq!(Some("new-client"), saved.client_id.as_deref());
    assert_eq!(Some(redirect.as_str()), saved.redirect_uri.as_deref());
    assert_eq!(Some("at-9"), saved.access_token.as_deref());
    assert_eq!(Some("rt-9"), saved.refresh_token.as_deref());
    // expires_in 不是整數：預設 3600 秒
    let left = saved.expires_at.with_timezone(&Utc) - Utc::now();
    assert!(left > TimeDelta::seconds(3500) && left <= TimeDelta::seconds(3600));
    assert!(auth.has_session());

    // 登入完回呼埠已關閉
    assert!(std::net::TcpListener::bind(("127.0.0.1", port)).is_ok());
}

#[test]
fn 登入_使用者拒絕() {
    let dir = tempfile::tempdir().unwrap();
    let server = MockServer::start(|req| match req.path.as_str() {
        "/.well-known/oauth-authorization-server" => meta_reply(req),
        _ => Reply::json(404, "{}"),
    });
    let port = free_port();
    let store = store_with(dir.path(), None, None, 0);
    // 已註冊過同一個回呼網址：不重新註冊
    let mut data = token_store::load_from(&store);
    data.redirect_uri = Some(format!("http://127.0.0.1:{port}/callback"));
    token_store::save_to(&store, &data).unwrap();
    let pages: Arc<Mutex<Vec<String>>> = Arc::default();
    let threads: Arc<Mutex<Vec<std::thread::JoinHandle<()>>>> = Arc::default();
    let (p, t) = (pages.clone(), threads.clone());
    let auth = OAuthClient::build(
        crate::http::app_client(),
        &server.base_url,
        &store,
        port,
        Box::new(move |url| {
            let url = url.to_owned();
            let p = p.clone();
            let handle = std::thread::spawn(move || {
                p.lock().unwrap().push(browser(&url, port, None, Some("access_denied"), false))
            });
            t.lock().unwrap().push(handle);
            Ok(())
        }),
    );
    assert_eq!(Err(Error::InvalidOperation("授權失敗：access_denied 使用者 拒絕".into())), rt::block_on(auth.login()));
    for t in threads.lock().unwrap().drain(..) {
        t.join().unwrap();
    }
    assert!(server.requests().iter().all(|r| r.path != "/register"));
    assert!(pages.lock().unwrap()[0].contains("<h2>登入失敗</h2><p>access_denied</p>"));
}

#[test]
fn 登入_換token時client被撤銷_清掉登入資料() {
    let dir = tempfile::tempdir().unwrap();
    let server = MockServer::start(|req| match req.path.as_str() {
        "/.well-known/oauth-authorization-server" => meta_reply(req),
        "/token" => Reply::json(401, r#"{"error":"invalid_client"}"#),
        _ => Reply::json(404, "{}"),
    });
    let port = free_port();
    let store = store_with(dir.path(), None, Some("rt-old"), 0);
    let mut data = token_store::load_from(&store);
    data.redirect_uri = Some(format!("http://127.0.0.1:{port}/callback"));
    token_store::save_to(&store, &data).unwrap();
    let auth = OAuthClient::build(
        crate::http::app_client(),
        &server.base_url,
        &store,
        port,
        Box::new(move |url| {
            let url = url.to_owned();
            std::thread::spawn(move || browser(&url, port, Some("c"), None, false));
            Ok(())
        }),
    );
    assert_eq!(
        Err(Error::InvalidOperation("換取 token 失敗（401）：{\"error\":\"invalid_client\"}".into())),
        rt::block_on(auth.login())
    );
    assert_eq!(AuthData::default(), token_store::load_from(&store));
}

#[test]
fn 登入_回呼埠被占用() {
    let dir = tempfile::tempdir().unwrap();
    let server = MockServer::start(meta_reply);
    let busy = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = busy.local_addr().unwrap().port();
    let store = store_with(dir.path(), None, None, 0);
    let mut data = token_store::load_from(&store);
    data.redirect_uri = Some(format!("http://127.0.0.1:{port}/callback"));
    token_store::save_to(&store, &data).unwrap();
    let auth = OAuthClient::build(crate::http::app_client(), &server.base_url, &store, port, Box::new(|_| Ok(())));
    assert_eq!(
        Err(Error::InvalidOperation(format!("本機連接埠 {port} 被占用，無法接收登入結果"))),
        rt::block_on(auth.login())
    );
}

#[test]
fn 登出_撤銷refresh_token並清掉() {
    let dir = tempfile::tempdir().unwrap();
    let server = MockServer::start(|req| match req.path.as_str() {
        "/.well-known/oauth-authorization-server" => meta_reply(req),
        _ => Reply::json(200, "{}"),
    });
    let store = store_with(dir.path(), Some("at-1"), Some("rt-1"), 3600);
    let auth = client(&server.base_url, &store);
    rt::block_on(auth.logout());
    let revoke = server.requests().into_iter().find(|r| r.path == "/revoke").unwrap();
    let f = form(&revoke.body);
    assert_eq!(
        ("rt-1", "refresh_token", "cid"),
        (f["token"].as_str(), f["token_type_hint"].as_str(), f["client_id"].as_str())
    );
    assert!(!auth.has_session());
    let saved = token_store::load_from(&store);
    assert_eq!((None, None, Some("cid")), (saved.access_token, saved.refresh_token, saved.client_id.as_deref()));
}

#[test]
fn 登出_伺服器連不上也會清掉() {
    let dir = tempfile::tempdir().unwrap();
    let store = store_with(dir.path(), Some("at-1"), Some("rt-1"), 3600);
    let auth = client(&format!("http://127.0.0.1:{}", free_port()), &store);
    rt::block_on(auth.logout());
    assert!(!auth.has_session());
}

#[test]
fn 中繼資料缺欄位() {
    let dir = tempfile::tempdir().unwrap();
    let server = MockServer::start(|_| Reply::json(200, "{}"));
    let store = store_with(dir.path(), None, Some("rt"), -1);
    let auth = client(&server.base_url, &store);
    assert_eq!(
        Err(Error::InvalidOperation("OAuth 設定缺少 token_endpoint".into())),
        rt::block_on(auth.get_access_token())
    );
}

#[test]
fn 查詢字串_解碼與同名合併() {
    let q = parse_query("code=a%2Bb&state=x+y&dup=1&dup=2&flag&error_description=%E4%B8%AD");
    assert_eq!(Some("a+b"), q.get("code"));
    assert_eq!(Some("x y"), q.get("state"));
    assert_eq!(Some("1,2"), q.get("dup"));
    assert_eq!(None, q.get("flag"));
    assert_eq!(Some("中"), q.get("error_description"));
}

#[test]
fn html_編碼跟httputility一樣() {
    assert_eq!("&lt;a href=&quot;x&quot;&gt;&amp;&#39;&#233;中", html_encode("<a href=\"x\">&'é中"));
}
