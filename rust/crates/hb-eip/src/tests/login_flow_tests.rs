//! 用本機假伺服器跑 EipClient 的登入與讀假單流程（不連真正的 EIP）。

use std::sync::{Arc, Mutex};

use hb_backend::infra::{MiniHttpServer, MiniRequest, MiniResponse};

use super::eip_client_tests::{IDENTITY_PAGE, LEAVE_LIST, LOGIN_PAGE, PLEASE_LOGIN_PAGE};
use crate::{EipClient, EipCredentials, EipError};

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

/// 別人的假單（主管會看到部屬的），要被 person_no 過濾掉。
fn leave_list_with_other() -> String {
    LEAVE_LIST.replace(
        "</tr>\t </table>",
        "</tr><tr><td></td><td>20261101-0001</td><td>Y0001</td><td>部屬</td><td>請假</td><td>特休</td><td></td>\
         <td>2026-11-03 08:30</td><td>2026-11-03 17:30</td><td>1天0小時</td><td></td><td></td><td>核准</td></tr>\t </table>",
    )
}

struct FakeEip {
    _server: MiniHttpServer,
    requests: Arc<Mutex<Vec<MiniRequest>>>,
    base: String,
}

/// list_pages：leave_list.php 依序回的內容（用完後重複最後一個）。
fn fake_eip(list_pages: Vec<String>) -> FakeEip {
    let requests: Arc<Mutex<Vec<MiniRequest>>> = Arc::default();
    let pages = Arc::new(Mutex::new(list_pages));
    let log = requests.clone();
    let port = free_port();
    let mut server = MiniHttpServer::new(port, move |req: MiniRequest| {
        log.lock().unwrap().push(req.clone());
        let pages = pages.clone();
        async move {
            let body = match (req.method.as_str(), req.path.as_str()) {
                ("GET", "/plug/home/php/login.php") => LOGIN_PAGE.to_owned(),
                ("POST", "/plug/home/php/login.php") if req.body.contains("password=wrong") => {
                    format!("{LOGIN_PAGE}<script>alert('帳號或密碼錯誤');</script>")
                }
                ("POST", "/plug/home/php/login.php") if req.body.contains("password=") => IDENTITY_PAGE.to_owned(),
                ("POST", "/plug/home/php/login.php") => {
                    "<html><frameset><frame src='main.php'></frameset></html>".into()
                }
                ("POST", "/plug/absentNext/php/leave_list.php") => {
                    let mut pages = pages.lock().unwrap();
                    if pages.len() > 1 { pages.remove(0) } else { pages[0].clone() }
                }
                _ => return Ok(MiniResponse::new(404, "text/plain", "not found")),
            };
            Ok(MiniResponse::new(200, "text/html", body))
        }
    });
    server.start().unwrap();
    FakeEip { _server: server, requests, base: format!("http://127.0.0.1:{port}/") }
}

#[test]
fn 兩段登入_欄位照原樣送回去() {
    let eip = fake_eip(vec![LEAVE_LIST.to_owned()]);
    let client = EipClient::with_base(&eip.base);
    let result = hb_backend::rt::block_on(client.login(&EipCredentials::new("Y2798", "p w")));

    assert!(result.as_ref().unwrap().ok, "{result:?}");
    assert_eq!("登入成功", result.unwrap().message);
    assert!(client.logged_in());
    assert_eq!(Some("Y2798"), client.person_no().as_deref());
    let requests = eip.requests.lock().unwrap();
    assert_eq!(3, requests.len());
    assert_eq!("get_url=", requests[0].query);
    assert_eq!(
        "csrf_token=cc8354cb156d9357afadb374801bfe70&state=Y2hlY2s%253D&account=Y2798&password=p+w",
        requests[1].body
    );
    assert_eq!(
        "account_no=Y2798&csrf_token=7446e1c086e9ddabe75249c09b3c6a5c&state=cG9zaXRpb24%253D&person_no=Y2798",
        requests[2].body
    );
}

#[test]
fn 帳密錯誤回ok_false與alert訊息() {
    let eip = fake_eip(vec![LEAVE_LIST.to_owned()]);
    let client = EipClient::with_base(&eip.base);
    let result = hb_backend::rt::block_on(client.login(&EipCredentials::new("Y2798", "wrong"))).unwrap();
    assert!(!result.ok);
    assert_eq!("帳號或密碼錯誤", result.message);
    assert!(!client.logged_in());

    // 讀假單時登入失敗：EipLoginException
    let err = hb_backend::rt::block_on(client.list_leaves(
        2026,
        EipClient::STATE_APPROVED,
        &EipCredentials::new("Y2798", "wrong"),
    ))
    .unwrap_err();
    assert_eq!(EipError::Login("EIP 帳號或密碼錯誤".into()), err);
}

#[test]
fn session失效時重新登入再讀_只留自己的假單() {
    assert_eq!(3, EipClient::parse_leave_list(&leave_list_with_other()).len());
    let eip = fake_eip(vec![PLEASE_LOGIN_PAGE.to_owned(), leave_list_with_other()]);
    let client = EipClient::with_base(&eip.base);
    let cred = EipCredentials::new("Y2798", "pw");
    hb_backend::rt::block_on(client.login(&cred)).unwrap();
    let leaves = hb_backend::rt::block_on(client.list_leaves(2026, EipClient::STATE_PENDING, &cred)).unwrap();

    assert_eq!(vec!["20261027-0078", "20261005-0008"], leaves.iter().map(|l| l.no.as_str()).collect::<Vec<_>>());
    let requests = eip.requests.lock().unwrap();
    // 登入 3 次請求 → 假單（請登入）→ 重新登入 3 次 → 假單
    assert_eq!(8, requests.len());
    assert_eq!("years=2026&selectDep=&selectPerson=&absent_state=0&btnSearch=", requests[3].body);
    assert_eq!("/plug/absentNext/php/leave_list.php", requests[7].path);
}

#[test]
fn 假單頁格式不對回錯誤() {
    let eip = fake_eip(vec!["<html>維護中</html>".to_owned()]);
    let client = EipClient::with_base(&eip.base);
    let err = hb_backend::rt::block_on(client.list_leaves(
        2026,
        EipClient::STATE_PENDING,
        &EipCredentials::new("Y2798", "pw"),
    ))
    .unwrap_err();
    assert_eq!(EipError::InvalidOperation("EIP 假單頁的格式跟預期不同，可能改版了".into()), err);
}

#[test]
fn 連不上回http錯誤() {
    let client = EipClient::with_base(&format!("http://127.0.0.1:{}/", free_port()));
    let err = hb_backend::rt::block_on(client.login(&EipCredentials::new("a", "b"))).unwrap_err();
    assert!(matches!(err, EipError::Http(_)), "{err:?}");
}
