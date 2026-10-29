using HayakuBar.Eip;

namespace HayakuBar.Tests;

public class EipClientTests
{
    // 2026-10-29 從 eip.yanray.com.tw 登入頁擷取的表單（節錄）
    const string LoginPage =
        "<form method='POST' action='./login.php?get_url=' style='margin:0'><table>" +
        "<tr><td><input type='text' name='account' size='20' maxlength='255' autocomplete='username' required></td></tr>" +
        "<tr><td><input type='password' name='password' size='20' autocomplete='current-password' onkeypress=\"detectCapsLock(event)\" required></td></tr>" +
        "<tr><td><input type='submit' value='登入'> <input type='reset' value='清除'></td></tr></table>" +
        "<input type='hidden' name='csrf_token' value='cc8354cb156d9357afadb374801bfe70'><input type='hidden' name='state' value='Y2hlY2s%3D'></form>";

    [Fact]
    public void HiddenFields_取出csrf與state_值照原樣()
    {
        var fields = EipClient.HiddenFields(LoginPage);

        Assert.Equal(2, fields.Count);
        Assert.Equal("cc8354cb156d9357afadb374801bfe70", fields["csrf_token"]);
        Assert.Equal("Y2hlY2s%3D", fields["state"]);
    }

    [Fact]
    public void HiddenFields_雙引號與大小寫也認得()
    {
        var fields = EipClient.HiddenFields("<INPUT TYPE=\"Hidden\" NAME=\"id\" VALUE=\"a&amp;b\"><input type=hidden name=x value=1>");

        Assert.Equal("a&b", fields["id"]);
        Assert.Equal("1", fields["x"]);
    }

    [Fact]
    public void IsLoginForm_有密碼欄位才算登入頁()
    {
        Assert.True(EipClient.IsLoginForm(LoginPage));
        Assert.False(EipClient.IsLoginForm("<html><frameset><frame src='main.php'></frameset></html>"));
        Assert.False(EipClient.IsLoginForm("<input type='text' name='password_hint'>"));
    }

    [Fact]
    public void AlertText_取出錯誤訊息()
    {
        Assert.Equal("帳號或密碼錯誤", EipClient.AlertText("<script>alert('帳號或密碼錯誤');history.back();</script>"));
        Assert.Equal("請重新登入", EipClient.AlertText("<script>alert( \"請重新登入\" )</script>"));
        Assert.Null(EipClient.AlertText(LoginPage));
    }

    // 2026-10-29 從「登入身分選擇」頁擷取（節錄）
    const string IdentityPage =
        "<form method='POST' name='login_it' action='login.php?get_url='><input type='hidden' name='account_no' value='Y2798'>" +
        "<input type='hidden' name='csrf_token' value='7446e1c086e9ddabe75249c09b3c6a5c'><input type='hidden' name='state' value='cG9zaXRpb24%3D'>" +
        "<table><input type='hidden' name='person_no' value='Y2798'><tr><td><input type='radio' checked name='person_no' value='Y2798'></td>" +
        "<td><input type='radio' name='other' value='X'></td></tr></table></form><script>document.forms['login_it'].submit();</script>";

    [Fact]
    public void IdentityFields_隱藏欄位加上勾選的radio()
    {
        var fields = EipClient.IdentityFields(IdentityPage);

        Assert.False(EipClient.IsLoginForm(IdentityPage));
        Assert.Equal("Y2798", fields["person_no"]);
        Assert.Equal("cG9zaXRpb24%3D", fields["state"]);
        Assert.Equal("7446e1c086e9ddabe75249c09b3c6a5c", fields["csrf_token"]);
        Assert.False(fields.ContainsKey("other"));
    }

    // 2026-10-29 從 leave_list.php 擷取（節錄，人名換掉）
    const string LeaveList = """
        <table class="table"><thead><tr class="active">
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
        </tr>	 </table>
        """;

    [Fact]
    public void ParseLeaveList_依表頭取欄位_略過沒有時間的黃卡()
    {
        var leaves = EipClient.ParseLeaveList(LeaveList);

        Assert.Equal(2, leaves.Count);
        var typhoon = leaves[0];
        Assert.Equal("20261027-0078", typhoon.No);
        Assert.Equal("Y2798", typhoon.PersonNo);
        Assert.Equal("請假", typhoon.Kind);
        Assert.Equal("天災假", typhoon.Type);
        Assert.Equal("颱風假", typhoon.Reason);
        Assert.Equal(new DateTime(2026, 10, 23, 8, 30, 0), typhoon.Start);
        Assert.Equal(new DateTime(2026, 10, 23, 17, 30, 0), typhoon.End);
        Assert.Equal("核准", typhoon.Status);
        Assert.Equal("190286", typhoon.Id);
        Assert.Equal("1天0小時", typhoon.Subtotal);
        Assert.Equal("回學校考試 & 報告", leaves[1].Reason);
        Assert.Equal("189401", leaves[1].Id);
    }

    [Fact]
    public void ParseLeaveList_沒有資料或格式不對回空清單()
    {
        Assert.Empty(EipClient.ParseLeaveList("<table><thead><tr><th>單號</th><th>請假時間(起)</th></tr></thead></table>"));
        Assert.Empty(EipClient.ParseLeaveList("<html>維護中</html>"));
    }
}
