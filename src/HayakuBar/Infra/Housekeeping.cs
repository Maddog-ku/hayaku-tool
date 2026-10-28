using System.Diagnostics;
using System.Runtime;
using System.Runtime.InteropServices;

namespace HayakuBar.Infra;

/// <summary>定期清暫存：刪掉舊的截圖與紀錄、整理記憶體（壓縮 GC heap、把用不到的記憶體還給 Windows）。</summary>
public static class Housekeeping
{
    static readonly TimeSpan ShotsKeep = TimeSpan.FromDays(3);
    static readonly TimeSpan OldLogKeep = TimeSpan.FromDays(14);

    [DllImport("psapi.dll")]
    static extern bool EmptyWorkingSet(IntPtr process);

    /// <summary>刪掉 3 天前的截圖（--shot 產生的）與 14 天前輪替下來的舊紀錄；回傳刪了幾個檔。</summary>
    public static int CleanFiles(string dir, DateTime now)
    {
        var deleted = 0;
        void Delete(FileInfo f)
        {
            try
            {
                f.Delete();
                deleted++;
            }
            catch (Exception ex) when (ex is IOException or UnauthorizedAccessException)
            {
                // 正在用的檔案下次再刪
            }
        }
        var shots = new DirectoryInfo(Path.Combine(dir, "shots"));
        if (shots.Exists)
            foreach (var f in shots.EnumerateFiles().Where(f => now - f.LastWriteTime > ShotsKeep).ToList())
                Delete(f);
        var oldLog = new FileInfo(Path.Combine(dir, "log.txt.1"));
        if (oldLog.Exists && now - oldLog.LastWriteTime > OldLogKeep)
            Delete(oldLog);
        return deleted;
    }

    /// <summary>壓縮 GC heap（含大型物件），再把閒置的實體記憶體還給 Windows（之後用到會自動拿回來）。</summary>
    public static void TrimMemory()
    {
        GCSettings.LargeObjectHeapCompactionMode = GCLargeObjectHeapCompactionMode.CompactOnce;
        GC.Collect(GC.MaxGeneration, GCCollectionMode.Aggressive, blocking: true, compacting: true);
        GC.WaitForPendingFinalizers();
        try
        {
            using var self = Process.GetCurrentProcess();
            EmptyWorkingSet(self.Handle);
        }
        catch (Exception ex) when (ex is InvalidOperationException or System.ComponentModel.Win32Exception or DllNotFoundException)
        {
            Log.Error("釋放記憶體失敗", ex);
        }
    }

    /// <summary>目前佔用的記憶體（工作集、私有），給紀錄用。</summary>
    public static string MemoryText()
    {
        using var self = Process.GetCurrentProcess();
        return $"工作集 {self.WorkingSet64 / 1048576} MB、私有 {self.PrivateMemorySize64 / 1048576} MB、GC heap {GC.GetTotalMemory(false) / 1048576} MB";
    }
}
