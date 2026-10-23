# Engine detection is read-only and runs on a .NET worker, never on the UI thread.
if (-not ('DlssgUnrealDetector' -as [type])) {
    Add-Type -TypeDefinition @"
using System;
using System.IO;
using System.Text;
using System.Text.RegularExpressions;
using System.Diagnostics;
using System.Threading.Tasks;
using System.Collections.Concurrent;
using System.Collections.Generic;
public sealed class DlssgUnrealResult {
    public int Major = 0;
    public string Source = "无法确定";
    public string Key;
}
public static class DlssgUnrealDetector {
    static readonly ConcurrentDictionary<string, Task<DlssgUnrealResult>> Cache = new ConcurrentDictionary<string, Task<DlssgUnrealResult>>();
    static string Stamp(string path) {
        var f = new FileInfo(path);
        return f.Exists ? f.FullName.ToLowerInvariant()+"|"+f.Length+"|"+f.LastWriteTimeUtc.Ticks : path+"|missing";
    }
    public static string Key(string game, string exe) {
        return Stamp(exe)+"|"+Stamp(Path.Combine(game,"Engine","Build","Build.version"));
    }
    public static Task<DlssgUnrealResult> Start(string game, string exe) {
        string key = Key(game,exe);
        return Cache.GetOrAdd(key, k => Task.Run(() => Scan(game,exe,k)));
    }
    static void Mark(string text, string source, Dictionary<int,List<string>> evidence) {
        foreach (Match m in Regex.Matches(text, @"\+\+UE([45])\+Release-([45])\.\d+(?:\.\d+)?")) {
            if (m.Groups[1].Value == m.Groups[2].Value) Add(int.Parse(m.Groups[1].Value),source+" "+m.Value,evidence);
        }
    }
    static void Add(int major, string source, Dictionary<int,List<string>> evidence) {
        if (major != 4 && major != 5) return;
        if (!evidence.ContainsKey(major)) evidence[major]=new List<string>();
        if (!evidence[major].Contains(source)) evidence[major].Add(source);
    }
    static DlssgUnrealResult Scan(string game, string exe, string key) {
        var result=new DlssgUnrealResult { Key=key };
        var evidence=new Dictionary<int,List<string>>();
        try {
            string build=Path.Combine(game,"Engine","Build","Build.version");
            if (File.Exists(build)) {
                if (new FileInfo(build).Length > 1048576) throw new IOException("Build.version过大");
                string json=File.ReadAllText(build);
                var m=Regex.Match(json,"\\\"MajorVersion\\\"\\s*:\\s*([0-9]+)");
                if (m.Success) Add(int.Parse(m.Groups[1].Value),"Build.version",evidence);
            }
            var v=FileVersionInfo.GetVersionInfo(exe);
            foreach (string text in new [] { v.ProductVersion, v.FileVersion, v.ProductName, v.FileDescription }) {
                if (text == null) continue;
                Mark(text,"EXE版本信息",evidence);
                var m=Regex.Match(text,@"Unreal\s*Engine\s*([45])\.\d+",RegexOptions.IgnoreCase);
                if (m.Success) Add(int.Parse(m.Groups[1].Value),"EXE Unreal版本信息",evidence);
            }
            if (Regex.IsMatch((v.ProductName??"")+" "+(v.FileDescription??""),@"Unreal\s*Engine",RegexOptions.IgnoreCase)) {
                foreach (string text in new [] { v.ProductVersion, v.FileVersion }) {
                    var m=Regex.Match(text??"",@"^([45])\.\d+");
                    if (m.Success) Add(int.Parse(m.Groups[1].Value),"EXE Unreal版本信息",evidence);
                }
            }
            // Overlap preserves ASCII/UTF-16 markers crossing chunk boundaries.
            byte[] buffer=new byte[1048576+256]; int tail=0;
            using(var stream=new FileStream(exe,FileMode.Open,FileAccess.Read,FileShare.ReadWrite|FileShare.Delete)) {
                int read;
                while ((read=stream.Read(buffer,tail,1048576))>0) {
                    int count=tail+read;
                    Mark(Encoding.ASCII.GetString(buffer,0,count),"EXE标记",evidence);
                    Mark(Encoding.Unicode.GetString(buffer,0,count-count%2),"EXE标记",evidence);
                    if(count>1) Mark(Encoding.Unicode.GetString(buffer,1,(count-1)-(count-1)%2),"EXE标记",evidence);
                    tail=Math.Min(256,count); Array.Copy(buffer,count-tail,buffer,0,tail);
                }
            }
            if (key != Key(game,exe)) { result.Source="识别期间文件发生变化，请重新查找"; return result; }
            if(evidence.Count==1) foreach(var pair in evidence) { result.Major=pair.Key; result.Source=string.Join("；",pair.Value); }
            else if(evidence.Count>1) result.Source="UE4/UE5识别证据冲突，请手动校准";
        } catch(Exception e) { result.Source="无法确定："+e.Message; }
        return result;
    }
}
"@
}

function Initialize-PortableQuality {
    param($Profile)
    if ($null -eq $Profile.PSObject.Properties['QualityPreset']) { $Profile | Add-Member NoteProperty QualityPreset Current }
    if ($null -eq $Profile.PSObject.Properties['EngineOverride']) { $Profile | Add-Member NoteProperty EngineOverride Auto }
    if ($null -eq $Profile.PSObject.Properties['DetectedEngineMajor']) { $Profile | Add-Member NoteProperty DetectedEngineMajor 0 }
    if ($null -eq $Profile.PSObject.Properties['DetectedEngineSource']) { $Profile | Add-Member NoteProperty DetectedEngineSource '' }
    if ($Profile.QualityPreset -notin @('Current','Tier1','Tier2','Tier3','Tier4','Tier5')) { throw '画质档位无效。' }
    if ($Profile.EngineOverride -notin @('Auto','UE4','UE5')) { throw '引擎校准设置无效。' }
}

function Resolve-PortableQuality {
    param($Profile, $Detection, [string]$Root=(Get-LauncherRoot))
    Initialize-PortableQuality $Profile
    if ($null -eq $Detection) { $Detection=Get-PortableSavedEngine $Profile }
    $major=0; $source='无法确定'
    if ($null -ne $Detection) { $major=[int]$Detection.Major; $source=[string]$Detection.Source }
    if ($Profile.EngineOverride -ne 'Auto') { $major=[int]$Profile.EngineOverride.Substring(2); $source='手动校准' }
    $path=Resolve-LauncherConfigPath $Profile.EngineConfigSource '配置源' -Root $Root
    if ($Profile.QualityPreset -ne 'Current') {
        if ($major -notin @(4,5)) { throw '尚未确定UE4/UE5，请等待识别或在高级设置校准引擎版本。' }
        $folder=@('01-高性能均衡','02-主流均衡','03-性能优先','04-入门流畅','05-自定义')[[int]$Profile.QualityPreset.Substring(4)-1]
        $path=Join-Path $Root "配置\虚幻通用分类\UE$major\$folder\Engine.ini"
    }
    return [pscustomobject]@{Major=$major;Source=$source;Path=$path}
}

# Commit a copied profile only after durable preference writing succeeds.
function Save-PortableProfileChange {
    param($Preferences,$Profile,[string]$Directory=(Get-PortableDataDirectory))
    $old=$Preferences.Profiles
    try { Set-PortableProfile $Preferences $Profile; Save-PortablePreferences $Preferences -Directory $Directory }
    catch { $Preferences.Profiles=$old; throw }
}

function Get-PortableSavedEngine {
    param($Profile)
    if ($null -eq $Profile.PSObject.Properties['DetectedEngineMajor'] -or [string]$Profile.DetectedEngineMajor -notmatch '^[45]$') { return $null }
    $source='已保存的识别结果'
    if ($null -ne $Profile.PSObject.Properties['DetectedEngineSource'] -and $Profile.DetectedEngineSource) { $source+='：'+[string]$Profile.DetectedEngineSource }
    return [pscustomobject]@{Major=[int]$Profile.DetectedEngineMajor;Source=$source}
}

function Save-PortableEngineDetection {
    param($Preferences,$Profile,$Detection,[string]$Directory=(Get-PortableDataDirectory))
    if ($null -eq $Detection -or [string]$Detection.Major -notmatch '^[45]$') { return }
    $copy=$Profile | ConvertTo-Json -Depth 6 | ConvertFrom-Json
    Initialize-PortableQuality $copy
    $copy.DetectedEngineMajor=[int]$Detection.Major
    $copy.DetectedEngineSource=[string]$Detection.Source
    Save-PortableProfileChange $Preferences $copy -Directory $Directory
    # Publish to the selected UI object only after the atomic write succeeds.
    Initialize-PortableQuality $Profile
    $Profile.DetectedEngineMajor=$copy.DetectedEngineMajor
    $Profile.DetectedEngineSource=$copy.DetectedEngineSource
}
