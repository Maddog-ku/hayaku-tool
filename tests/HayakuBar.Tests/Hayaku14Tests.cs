using System.Text.Json;
using HayakuBar.Mcp;
using HayakuBar.UI;

namespace HayakuBar.Tests;

/// <summary>
/// Hayaku 1.4 的新欄位與工具：測試子狀態（testSubStatuses、testStage）、資料夾（取代檔案庫）、搜尋到檔案、出差、
/// 通知的討論分頁、whoami 的版本。
/// 子狀態設定、測試總覽、討論分頁、請假列表取自實際回應；資料夾與搜尋到檔案還沒有樣本（帳號看不到空間），照工具說明寫。
/// </summary>
public class Hayaku14Tests
{
    static JsonElement Json(string s) => JsonDocument.Parse(s).RootElement.Clone();

    [Fact]
    public void 專案的測試子狀態_記下所屬階段_分組照階段()
    {
        var subs = HayakuApi.ParseSubStatuses(Json("""
            {"testSubStatuses":[
              {"stage":"修復中","statusId":"a","name":"修復中","color":"#FA8C16","batchLabel":null},
              {"stage":"修復中","statusId":"b","name":"待更新測試機","color":"#9C7A3C","batchLabel":"更新到測試機"},
              {"stage":"待復驗","statusId":"c","name":"待復驗","color":"#722ED1","batchLabel":null},
              {"stage":"待復驗","statusId":"d","name":"待更新正式機","color":"#52C41A","batchLabel":"更新正式機"}]}
            """), "testSubStatuses");
        Assert.Equal(["修復中", "待更新測試機", "待復驗", "待更新正式機"], subs.Select(s => s.Name));
        Assert.Equal("已更新到測試機", subs[1].BatchText);
        Assert.Null(subs[0].BatchText);

        // 待更新正式機 是 IN_PROGRESS 類別，以前會被當成修復中；現在照階段算待復驗
        Assert.Equal("retest", TaskKinds.Bucket("待更新正式機", "IN_PROGRESS"));
        Assert.Equal("fixing", TaskKinds.Bucket("待更新測試機", "IN_PROGRESS"));
        // 回應有 testStage 時以它為準
        Assert.Equal("retest", TaskKinds.Bucket("待上正式機", "IN_PROGRESS", "待復驗"));
        Assert.Equal("待復驗", TaskKinds.StageOf("待更新正式機"));
    }

    [Fact]
    public void 測試結果列_讀_testStage()
    {
        var rows = HayakuApi.ParseTestRows(Json("""
            {"view":"需修復任務","results":[{"task":"TEST-148","title":"按鈕被擋住","status":"待上線","testStage":"待復驗","type":"測試結果","round":2}]}
            """));
        Assert.Equal("待復驗", rows[0].Stage);
        Assert.Equal("待復驗", rows[0].StageName);
    }

    [Fact]
    public void 測試總覽_子狀態數量與待上線()
    {
        var o = HayakuApi.ParseTestOverview(Json("""
            {"cards":{"open":{"total":11},"retest":{"total":0},"created":{"count":97},"closed":{"done":86,"wontfix":0,"pendingRelease":3}},
             "subStatuses":[{"stage":"修復中","name":"待更新測試機","count":2,"oldestDays":1,"batchLabel":"更新到測試機"},
                            {"stage":"待復驗","name":"待更新正式機","count":0,"oldestDays":null,"batchLabel":"更新正式機"}]}
            """));
        Assert.Equal(3, o.PendingRelease);
        Assert.Equal(2, o.SubStatuses!.Count);
        Assert.Equal((2, 1), (o.SubStatuses[0].Count ?? 0, o.SubStatuses[0].OldestDays ?? 0));
    }

    [Fact]
    public void 空間頁面樹_資料夾與檔案數()
    {
        var nodes = HayakuApi.ParseSpaceTree(Json("""
            {"pages":[{"id":"f","title":"設計稿","kind":"FOLDER","fileCount":4,"children":[{"id":"d","title":"說明","kind":"DOC"}]},
                      {"id":"b","title":"白板","kind":"BOARD"}]}
            """));
        Assert.True(nodes[0].IsFolder);
        Assert.Equal(4, nodes[0].FileCount);
        Assert.False(nodes[1].IsFolder);
        Assert.Equal("f", nodes[1].ParentId);
        Assert.Equal("🖍 ", nodes[2].Icon);
    }

    [Fact]
    public void 資料夾內容_子頁在前_檔案在後()
    {
        var items = HayakuApi.ParseSpaceFiles(Json("""
            {"pages":[{"id":"d","title":"說明","kind":"DOC"},{"id":"s","title":"子資料夾","kind":"FOLDER"}],
             "files":[{"id":"x","name":"a.pdf","size":1024,"uploader":{"displayName":"小明"},"path":"設計稿 › 子資料夾"}]}
            """));
        Assert.Equal(["s", "d", "x"], items.Select(i => i.Id));
        Assert.True(items[0].IsFolder);
        Assert.True(items[1].IsPage);
        Assert.False(items[1].IsFolder);
        Assert.False(items[2].IsPage);
        Assert.Equal("設計稿 › 子資料夾", items[2].Path);

        // 舊的檔案庫回應（folders＋files）照樣讀得到
        var old = HayakuApi.ParseSpaceFiles(Json("""{"folders":[{"id":"o","name":"舊資料夾"}],"files":[{"id":"y","name":"b.txt"}]}"""));
        Assert.Equal(["o", "y"], old.Select(i => i.Id));
        Assert.True(old[0].IsFolder);
    }

    [Fact]
    public void 搜尋空間_檔案結果帶所在資料夾()
    {
        var hits = HayakuApi.ParseFileHits(Json("""
            {"results":[{"pageId":"p","title":"規範"}],"files":[{"id":"x","name":"報價.xlsx","folderId":"f","path":["業務","報價"]}]}
            """));
        var hit = Assert.Single(hits);
        Assert.True(hit.IsFile);
        Assert.Equal(("x", "f", "業務 › 報價"), (hit.Id, hit.FolderId, hit.Snippet));
    }

    [Fact]
    public void 請假列表_分得出出差()
    {
        var leaves = HayakuApi.ParseLeaves(Json("""
            {"leaves":[
              {"id":"1","user":{"username":"a","displayName":"甲"},"kind":"請假","period":"11/9 整天","startAt":"2026-11-08T16:00:00Z","endAt":"2026-11-09T16:00:00Z","lost":{"display":"8 小時"}},
              {"id":"2","user":{"username":"b","displayName":"乙"},"kind":"出差","period":"11/10 整天","startAt":"2026-11-09T16:00:00Z","endAt":"2026-11-10T16:00:00Z"}]}
            """));
        Assert.False(leaves[0].IsTrip);
        Assert.True(leaves[1].IsTrip);
        Assert.False(new LeaveItem("3", new PersonRef("c", "丙"), "", default, default, "").IsTrip); // 舊的回應沒有 kind
    }

    [Fact]
    public void 通知的討論分頁_一段討論一組()
    {
        var list = HayakuApi.ParseNotifications(Json("""
            {"items":[{"latest":{"id":"n2","type":"MENTIONED","actor":{"displayName":"羅瑞騰"},
                         "project":{"key":"TEST"},"task":{"number":11,"title":"42","kind":"TASK"},
                         "payload":{"task":"TEST-11 42","excerpt":"@harry.gu 你做好了嗎？"},"readAt":null,"createdAt":"2026-10-27T07:51:11.294Z"},
                       "count":2,"unreadCount":1,"ids":["n1","n2"],"users":["羅瑞騰","吳景涵"],"resolved":false,"rootExcerpt":null}]}
            """));
        var n = Assert.Single(list);
        Assert.Equal(["n1", "n2"], n.AllIds);
        Assert.Equal(["羅瑞騰", "吳景涵"], n.Users!);
        Assert.False(n.Resolved);
        Assert.Equal("@harry.gu 你做好了嗎？", n.Quote);
        Assert.True(n.Unread);

        var vm = new NotificationRowVm { Item = n, Discussion = true };
        Assert.Contains("「@harry.gu 你做好了嗎？」", vm.Text);
        Assert.StartsWith("羅瑞騰、吳景涵", vm.Meta);
        // 收件匣照舊只顯示任務
        Assert.Equal("TEST-11 42", new NotificationRowVm { Item = n }.Text);
    }

    [Fact]
    public void 收件匣通知沒有_ids_時_AllIds_是自己() =>
        Assert.Equal(["n"], new NotificationItem("n", "TASK_ASSIGNED", "", "", null, null, null, null, default, true, 1).AllIds);
}
