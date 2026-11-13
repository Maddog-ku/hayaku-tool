//! 移植自 EipClientTests.cs。

use chrono::NaiveDate;

use crate::EipClient;

// 2026-10-29 從 eip.yanray.com.tw 登入頁擷取的表單（節錄）
pub(crate) const LOGIN_PAGE: &str = concat!(
    "<form method='POST' action='./login.php?get_url=' style='margin:0'><table>",
    "<tr><td><input type='text' name='account' size='20' maxlength='255' autocomplete='username' required></td></tr>",
    "<tr><td><input type='password' name='password' size='20' autocomplete='current-password' onkeypress=\"detectCapsLock(event)\" required></td></tr>",
    "<tr><td><input type='submit' value='登入'> <input type='reset' value='清除'></td></tr></table>",
    "<input type='hidden' name='csrf_token' value='cc8354cb156d9357afadb374801bfe70'><input type='hidden' name='state' value='Y2hlY2s%3D'></form>"
);

#[test]
fn hidden_fields_取出csrf與state_值照原樣() {
    let fields = EipClient::hidden_fields(LOGIN_PAGE);

    assert_eq!(2, fields.len());
    assert_eq!("cc8354cb156d9357afadb374801bfe70", fields["csrf_token"]);
    assert_eq!("Y2hlY2s%3D", fields["state"]);
}

#[test]
fn hidden_fields_雙引號與大小寫也認得() {
    let fields = EipClient::hidden_fields(
        "<INPUT TYPE=\"Hidden\" NAME=\"id\" VALUE=\"a&amp;b\"><input type=hidden name=x value=1>",
    );

    assert_eq!("a&b", fields["id"]);
    assert_eq!("1", fields["x"]);
}

#[test]
fn is_login_form_有密碼欄位才算登入頁() {
    assert!(EipClient::is_login_form(LOGIN_PAGE));
    assert!(!EipClient::is_login_form("<html><frameset><frame src='main.php'></frameset></html>"));
    assert!(!EipClient::is_login_form("<input type='text' name='password_hint'>"));
}

// 2026-11-11 session 失效時 leave_list.php 回的整頁內容
pub(crate) const PLEASE_LOGIN_PAGE: &str = concat!(
    "<script>\n            alert('請登入');\n",
    "            location.href='/plug/home/php/login.php?get_url=L3BsdWcvYWJzZW50TmV4dC9waHAvbGVhdmVfbGlzdC5waHA%3D';\n        </script>"
);

#[test]
fn needs_login_登入頁或只有導去登入頁的script都算() {
    assert!(EipClient::needs_login(LOGIN_PAGE));
    assert!(EipClient::needs_login(PLEASE_LOGIN_PAGE));
    assert!(!EipClient::is_login_form(PLEASE_LOGIN_PAGE));
}

#[test]
fn needs_login_正常頁面裡有導去登入頁的script不算() {
    // 登入後的假單頁本來就有導去 login.php 的 script，不能當成 session 失效
    let page = concat!(
        "<html><body><script>function logout(){ location.href='/plug/home/php/login.php?get_url='; }</script>",
        "<table><tr><th>單號</th><th>請假時間(起)</th></tr></table></body></html>"
    );
    assert!(!EipClient::needs_login(page));
    assert!(!EipClient::needs_login("<script>alert('查無資料');history.back();</script>"));
    assert!(!EipClient::needs_login(""));
}

#[test]
fn alert_text_取出錯誤訊息() {
    assert_eq!(
        Some("帳號或密碼錯誤"),
        EipClient::alert_text("<script>alert('帳號或密碼錯誤');history.back();</script>").as_deref()
    );
    assert_eq!(Some("請重新登入"), EipClient::alert_text("<script>alert( \"請重新登入\" )</script>").as_deref());
    assert_eq!(None, EipClient::alert_text(LOGIN_PAGE));
}

// 2026-10-29 從「登入身分選擇」頁擷取（節錄）
pub(crate) const IDENTITY_PAGE: &str = concat!(
    "<form method='POST' name='login_it' action='login.php?get_url='><input type='hidden' name='account_no' value='Y2798'>",
    "<input type='hidden' name='csrf_token' value='7446e1c086e9ddabe75249c09b3c6a5c'><input type='hidden' name='state' value='cG9zaXRpb24%3D'>",
    "<table><input type='hidden' name='person_no' value='Y2798'><tr><td><input type='radio' checked name='person_no' value='Y2798'></td>",
    "<td><input type='radio' name='other' value='X'></td></tr></table></form><script>document.forms['login_it'].submit();</script>"
);

#[test]
fn identity_fields_隱藏欄位加上勾選的radio() {
    let fields = EipClient::identity_fields(IDENTITY_PAGE);

    assert!(!EipClient::is_login_form(IDENTITY_PAGE));
    assert_eq!("Y2798", fields["person_no"]);
    assert_eq!("cG9zaXRpb24%3D", fields["state"]);
    assert_eq!("7446e1c086e9ddabe75249c09b3c6a5c", fields["csrf_token"]);
    assert!(!fields.contains_key("other"));
}

// 2026-10-29 從 leave_list.php 擷取（節錄，人名換掉）
pub(crate) const LEAVE_LIST: &str = r#"<table class="table"><thead><tr class="active">
<th><label><input type="checkbox" id="checkAll"> 全選</label></th>
<th>單號</th><th>工號</th><th>姓名</th><th>類別</th><th>假別</th><th>請假原因</th>
<th>請假時間(起)</th><th>請假時間(迄)</th><th>小計</th><th>流程</th><th>審核人</th><th>狀態</th>
<th>審核狀態</th><th>審核</th><th>證明</th>
</tr></thead>
 <tr >
	<td></td><td>20261027-0078</td><td>Y2798</td><td>王小明</td><td >請假</td><td >天災假</td><td>颱風假</td>
	<td>2026-10-23 08:30</td><td>2026-10-23 17:30</td><td>1天0小時</td><td>單位主管</td><td>陳主管</td>
	<td title='颱風假'>核准</td>
	<td><input type="button" name="view" value="查看" id="MTkwMjg2" class="btn btn-default btn-xs view_data"></td>
	<td><a href="absent?state=YWJzZW50Vmlldw%3D%3D&ID=190286&S_PERSON_NAME=&S_VERIFY=0&S_STATE=0" class="btn">開啟</a></td><td></td>
</tr><tr >
	<td></td><td>20261005-0008</td><td>Y2798</td><td>王小明</td><td >請假</td><td >事假</td><td>回學校考試 &amp; 報告</td>
	<td>2026-10-06 15:30</td><td>2026-10-06 17:30</td><td>0天2小時</td><td>單位主管</td><td>陳主管</td>
	<td title='x'>核准</td><td></td><td><a href="absent?ID=189401">開啟</a></td><td></td>
</tr><tr >
	<td></td><td>20260903-0020</td><td>Y2798</td><td>王小明</td><td style='color:#f1c40f;'>黃卡</td><td ></td><td>新人報到</td>
	<td></td><td></td><td></td><td></td><td>陳主管</td><td title='新人報到'></td><td></td><td></td><td></td>
</tr>	 </table>"#;

#[test]
fn parse_leave_list_依表頭取欄位_略過沒有時間的黃卡() {
    let leaves = EipClient::parse_leave_list(LEAVE_LIST);

    assert_eq!(2, leaves.len());
    let typhoon = &leaves[0];
    assert_eq!("20261027-0078", typhoon.no);
    assert_eq!("Y2798", typhoon.person_no);
    assert_eq!("請假", typhoon.kind);
    assert_eq!("天災假", typhoon.r#type);
    assert_eq!("颱風假", typhoon.reason);
    let d = NaiveDate::from_ymd_opt(2026, 10, 23).unwrap();
    assert_eq!(d.and_hms_opt(8, 30, 0).unwrap(), typhoon.start);
    assert_eq!(d.and_hms_opt(17, 30, 0).unwrap(), typhoon.end);
    assert_eq!("核准", typhoon.status);
    assert_eq!(Some("190286"), typhoon.id.as_deref());
    assert_eq!("1天0小時", typhoon.subtotal);
    assert_eq!("回學校考試 & 報告", leaves[1].reason);
    assert_eq!(Some("189401"), leaves[1].id.as_deref());
}

#[test]
fn parse_leave_list_沒有資料或格式不對回空清單() {
    assert!(
        EipClient::parse_leave_list("<table><thead><tr><th>單號</th><th>請假時間(起)</th></tr></thead></table>")
            .is_empty()
    );
    assert!(EipClient::parse_leave_list("<html>維護中</html>").is_empty());
}
