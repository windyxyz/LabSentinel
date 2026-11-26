# 仅检查本仓库的历史验收归档。默认预览，加 -Apply 删除匹配的重复文件。
[CmdletBinding(SupportsShouldProcess)]
param(
    [switch]$Apply,
    [switch]$List
)

$ErrorActionPreference = 'Stop'
$project = Split-Path -Parent $PSScriptRoot
if (!(Test-Path -LiteralPath (Join-Path $project 'engine-rs\Cargo.toml') -PathType Leaf) -or
    !(Test-Path -LiteralPath (Join-Path $project 'VERSION') -PathType Leaf)) {
    throw '请在完整的 LabSentinel 项目中运行此脚本。'
}

$resources = Join-Path $project 'Resources'
$archive = Join-Path $resources '验收归档'
$protected = Join-Path $archive '发布 - 0.9.6 最终双包核验'
$reparse = [IO.FileAttributes]::ReparsePoint
foreach ($path in @($project, $resources, $archive)) {
    $folder = Get-Item -LiteralPath $path -Force
    if (!$folder.PSIsContainer -or ($folder.Attributes -band $reparse)) {
        throw "归档路径必须是普通目录。$path"
    }
}

$prefix = $archive + [IO.Path]::DirectorySeparatorChar
$extensions = @('.exe', '.dll', '.onnx', '.bin', '.xz', '.pt', '.gguf', '.safetensors')
$folders = New-Object 'System.Collections.Generic.Stack[System.IO.DirectoryInfo]'
$folders.Push((Get-Item -LiteralPath $archive))
$candidates = while ($folders.Count) {
    foreach ($item in (Get-ChildItem -LiteralPath $folders.Pop().FullName -Force)) {
        if ($item.Attributes -band $reparse) { continue }
        if ($item.PSIsContainer) {
            $folders.Push($item)
        } elseif ($item.Length -ge 512KB -and
                  ($item.Extension -in $extensions -or $item.Name -match '^fixture(?:-\d+)?\.mp4$')) {
            $item
        }
    }
}

function Get-StreamSha256([IO.Stream]$Stream) {
    $sha256 = [Security.Cryptography.SHA256]::Create()
    try {
        [BitConverter]::ToString($sha256.ComputeHash($Stream)).Replace('-', '')
    } finally {
        $sha256.Dispose()
    }
}

function Get-PathSha256([string]$Path) {
    $stream = [IO.File]::OpenRead($Path)
    try {
        Get-StreamSha256 $stream
    } finally {
        $stream.Dispose()
    }
}

function Get-CheckedFile([string]$Path) {
    if (!$Path.StartsWith($prefix, [StringComparison]::OrdinalIgnoreCase)) {
        throw "文件路径超出验收归档。$Path"
    }
    $item = Get-Item -LiteralPath $Path -Force
    if ($item.PSIsContainer -or ($item.Attributes -band $reparse)) {
        throw "文件类型已改变。$Path"
    }
    $parent = $item.Directory
    while ($parent) {
        $parent.Refresh()
        if (!$parent.Exists -or ($parent.Attributes -band $reparse)) {
            throw "文件所在路径已改变。$Path"
        }
        if ($parent.FullName -eq $project) { return $item }
        $parent = $parent.Parent
    }
    throw "文件不在当前项目中。$Path"
}

$removed = 0
$planned = 0
$logicalBytes = 0L
foreach ($sameSize in ($candidates | Group-Object -Property Length | Where-Object Count -gt 1)) {
    $hashed = foreach ($file in $sameSize.Group) {
        $digest = Get-PathSha256 $file.FullName
        [pscustomobject]@{
            Path = $file.FullName
            Hash = $digest
            Bytes = $file.Length
            Protected = $file.FullName.StartsWith($protected + '\', [StringComparison]::OrdinalIgnoreCase)
            Fixture = $file.Name -eq 'fixture.mp4'
        }
    }
    foreach ($sameHash in ($hashed | Group-Object -Property Hash | Where-Object Count -gt 1)) {
        $ranked = @($sameHash.Group | Sort-Object -Property @(
            @{Expression='Protected'; Descending=$true},
            @{Expression='Fixture'; Descending=$true},
            @{Expression={$_.Path.Length}},
            'Path'
        ))
        $keep = $ranked[0]
        foreach ($copy in ($ranked | Select-Object -Skip 1)) {
            if ($copy.Protected -or $copy.Fixture) { continue }
            $planned++
            $logicalBytes += $copy.Bytes
            if ($List) {
                "重复文件 $($copy.Path)"
                "保留副本 $($keep.Path)"
            }
            if (!$Apply -or !$PSCmdlet.ShouldProcess($copy.Path, '删除重复验收产物')) { continue }
            $original = Get-CheckedFile $keep.Path
            $duplicate = Get-CheckedFile $copy.Path
            $stream = [IO.File]::Open($original.FullName, 'Open', 'Read', 'Read')
            try {
                if ($original.Length -ne $duplicate.Length -or
                    (Get-StreamSha256 $stream) -ne $keep.Hash -or
                    (Get-PathSha256 $duplicate.FullName) -ne $keep.Hash) {
                    throw "文件在扫描后发生变化。$($duplicate.FullName)"
                }
                Remove-Item -LiteralPath $duplicate.FullName -ErrorAction Stop
                $removed++
            } finally {
                $stream.Dispose()
            }
        }
    }
}

"符合条件的重复文件 $planned 个，原始大小 {0:N2} GiB，已删除 $removed 个。" -f ($logicalBytes / 1GB)
if (!$Apply) { '当前为预览。确认后加 -Apply 执行。' }
