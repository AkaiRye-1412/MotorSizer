# Local document recognition helper (PDF / image -> OCR text + word boxes).
#
# Called by MotionSizer Studio when importing motor parameters. All processing
# happens on this machine; no file or recognition result is uploaded.
# Output is NDJSON: first line describes the document, then one line per page.
#
# Parameters:
#   -Path        PDF or image path
#   -Out         output NDJSON path (UTF-8, no BOM)
#   -StartPage   first PDF page (1-based; ignored for images)
#   -EndPage     last PDF page (0 = last page; ignored for images)
#   -Width       PDF render width in pixels
#   -CancelFile  optional; stop as soon as this file exists
#   -Language    optional OCR language tag; defaults to the user profile
#   -PathList    optional batch file: one "id<TAB>image-path" line per image
#   -KeepRenders keep rendered PDF pages and report their directory
#
# Keep this file ASCII-only: Windows PowerShell 5.1 reads BOM-less scripts as
# ANSI, which corrupts non-ASCII source text.

param(
    [string]$Path = "",
    [Parameter(Mandatory = $true)][string]$Out,
    [int]$StartPage = 1,
    [int]$EndPage = 0,
    [int]$Width = 1800,
    [string]$CancelFile = "",
    [string]$Language = "",
    [string]$PathList = "",
    [switch]$KeepRenders
)

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Runtime.WindowsRuntime

# Load the WinRT types explicitly so PowerShell can resolve the namespaces.
$null = [Windows.Media.Ocr.OcrEngine, Windows.Foundation, ContentType = WindowsRuntime]
$null = [Windows.Media.Ocr.OcrResult, Windows.Foundation, ContentType = WindowsRuntime]
$null = [Windows.Graphics.Imaging.BitmapDecoder, Windows.Graphics.Imaging, ContentType = WindowsRuntime]
$null = [Windows.Graphics.Imaging.SoftwareBitmap, Windows.Graphics.Imaging, ContentType = WindowsRuntime]
$null = [Windows.Storage.StorageFile, Windows.Storage, ContentType = WindowsRuntime]
$null = [Windows.Storage.StorageFolder, Windows.Storage, ContentType = WindowsRuntime]
$null = [Windows.Storage.Streams.IRandomAccessStream, Windows.Storage.Streams, ContentType = WindowsRuntime]
$null = [Windows.Data.Pdf.PdfDocument, Windows.Data.Pdf, ContentType = WindowsRuntime]
$null = [Windows.Data.Pdf.PdfPageRenderOptions, Windows.Data.Pdf, ContentType = WindowsRuntime]
$null = [Windows.Globalization.Language, Windows.Globalization, ContentType = WindowsRuntime]

$methods = [System.WindowsRuntimeSystemExtensions].GetMethods()
$asTaskGeneric = ($methods | Where-Object {
    $_.Name -eq 'AsTask' -and $_.GetParameters().Count -eq 1 -and
    $_.GetParameters()[0].ParameterType.Name -eq 'IAsyncOperation`1'
})[0]
$asTaskAction = ($methods | Where-Object {
    $_.Name -eq 'AsTask' -and $_.GetParameters().Count -eq 1 -and
    $_.GetParameters()[0].ParameterType.Name -eq 'IAsyncAction'
})[0]

# Diagnostics are written only when the host keeps its workspace; this keeps
# failed recognitions debuggable without changing normal behavior.
function Trace($message) {
    if ($env:MOTORSIZER_KEEP_IMPORT -ne $null) {
        $dir = Join-Path $env:TEMP "motionsizer-import"
        $line = (Get-Date -Format "HH:mm:ss.fff") + " " + $message + [Environment]::NewLine
        [System.IO.File]::AppendAllText((Join-Path $dir "ocr-trace.txt"), $line, [System.Text.Encoding]::UTF8)
    }
}

function Await($op, $type, $label) {
    Trace ("await " + $label)
    $task = $asTaskGeneric.MakeGenericMethod($type).Invoke($null, @($op))
    $task.Wait()
    Trace ("await done " + $label)
    return $task.Result
}

function AwaitAction($op, $label) {
    Trace ("action " + $label)
    $task = $asTaskAction.Invoke($null, @($op))
    $task.Wait()
    Trace ("action done " + $label)
}

function ShouldCancel {
    return ($CancelFile -ne "" -and (Test-Path -LiteralPath $CancelFile))
}

Trace ("start path=" + $Path + " list=" + $PathList)

function Ocr-File($filePath, $pageId) {
    $script:stage = "recognize page " + $pageId
    $file = Await ([Windows.Storage.StorageFile]::GetFileFromPathAsync($filePath)) ([Windows.Storage.StorageFile]) "open file"
    $stream = Await ($file.OpenAsync([Windows.Storage.FileAccessMode]::Read)) ([Windows.Storage.Streams.IRandomAccessStream]) "open stream"
    $decoder = Await ([Windows.Graphics.Imaging.BitmapDecoder]::CreateAsync($stream)) ([Windows.Graphics.Imaging.BitmapDecoder]) "create decoder"
    $bitmap = Await ($decoder.GetSoftwareBitmapAsync()) ([Windows.Graphics.Imaging.SoftwareBitmap]) "get bitmap"
    $result = Await ($engine.RecognizeAsync($bitmap)) ([Windows.Media.Ocr.OcrResult]) "recognize"

    $lines = New-Object System.Collections.ArrayList
    foreach ($line in $result.Lines) {
        $words = New-Object System.Collections.ArrayList
        foreach ($word in $line.Words) {
            $rect = $word.BoundingRect
            [void]$words.Add([pscustomobject]@{
                t = $word.Text
                x = [int][Math]::Round($rect.X)
                y = [int][Math]::Round($rect.Y)
                w = [int][Math]::Round($rect.Width)
                h = [int][Math]::Round($rect.Height)
            })
        }
        [void]$lines.Add([pscustomobject]@{ text = $line.Text; words = $words })
    }

    $pageJson = [pscustomobject]@{
        page = $pageId
        width = $bitmap.PixelWidth
        height = $bitmap.PixelHeight
        lines = $lines
    }
    $bitmap.Dispose()
    $stream.Dispose()
    return $pageJson
}

$stage = "load document"
try {
$tempDir = $null
$pageCount = 1
$source = $Path
$kind = 'image'
$firstPage = 1
$pagesToRead = New-Object System.Collections.ArrayList

if ($PathList -ne "") {
    # Batch mode: each line is "id<TAB>path".
    $kind = 'batch'
    $source = $PathList
    foreach ($line in [System.IO.File]::ReadAllLines($PathList, [System.Text.Encoding]::UTF8)) {
        if ($line.Trim() -eq "") { continue }
        $parts = $line -split "`t", 2
        if ($parts.Count -ne 2) { throw "invalid batch line: $line" }
        [void]$pagesToRead.Add([pscustomobject]@{ page = $parts[0]; file = $parts[1] })
    }
    $pageCount = $pagesToRead.Count
} else {
    if ($Path -eq "") { throw "either -Path or -PathList is required" }
    $extension = [System.IO.Path]::GetExtension($Path).ToLowerInvariant()
    if ($extension -eq '.pdf') {
        $kind = 'pdf'
        $pdfFile = Await ([Windows.Storage.StorageFile]::GetFileFromPathAsync($Path)) ([Windows.Storage.StorageFile])
        $document = Await ([Windows.Data.Pdf.PdfDocument]::LoadFromFileAsync($pdfFile)) ([Windows.Data.Pdf.PdfDocument])
        $pageCount = $document.PageCount
        $first = [Math]::Max(1, $StartPage)
        $last = if ($EndPage -gt 0) { [Math]::Min($pageCount, $EndPage) } else { $pageCount }
        if ($first -gt $last) { throw "invalid page range: $first-$last" }
        $firstPage = $first

        # Render into a per-run temp directory; deleted unless -KeepRenders is set.
        $stage = "render pages"
        $tempDir = Join-Path $env:TEMP ("motionsizer-ocr-" + [Guid]::NewGuid().ToString('N'))
        New-Item -ItemType Directory -Path $tempDir | Out-Null
        $folder = Await ([Windows.Storage.StorageFolder]::GetFolderFromPathAsync($tempDir)) ([Windows.Storage.StorageFolder])

        for ($i = $first - 1; $i -lt $last; $i++) {
            if (ShouldCancel) { throw "cancelled" }
            $page = $document.GetPage($i)
            $name = "p-{0:D4}.png" -f ($i + 1)
            $outFile = Await ($folder.CreateFileAsync($name, [Windows.Storage.CreationCollisionOption]::ReplaceExisting)) ([Windows.Storage.StorageFile])
            $stream = Await ($outFile.OpenAsync([Windows.Storage.FileAccessMode]::ReadWrite)) ([Windows.Storage.Streams.IRandomAccessStream])
            $renderOptions = [Windows.Data.Pdf.PdfPageRenderOptions]::new()
            $renderOptions.DestinationWidth = $Width
            AwaitAction ($page.RenderToStreamAsync($stream, $renderOptions))
            $stream.Dispose()
            $page.Dispose()
            [void]$pagesToRead.Add([pscustomobject]@{ page = $i + 1; file = (Join-Path $tempDir $name) })
        }
    } else {
        [void]$pagesToRead.Add([pscustomobject]@{ page = 1; file = $Path })
    }
}

$stage = "create engine"
if ($Language -ne "") {
    $engine = [Windows.Media.Ocr.OcrEngine]::TryCreateFromLanguage([Windows.Globalization.Language]::new($Language))
} else {
    $engine = [Windows.Media.Ocr.OcrEngine]::TryCreateFromUserProfileLanguages()
    if ($null -eq $engine) {
        $engine = [Windows.Media.Ocr.OcrEngine]::TryCreateFromLanguage([Windows.Globalization.Language]::new('zh-Hans'))
    }
}
if ($null -eq $engine) {
    throw "no Windows OCR language pack is available (install the Chinese (Simplified) language)"
}

$utf8 = [System.Text.UTF8Encoding]::new($false)
$writer = [System.IO.StreamWriter]::new($Out, $false, $utf8)
try {
    $meta = [pscustomobject]@{
        source = $source
        kind = $kind
        page_count = $pageCount
        engine = $engine.RecognizerLanguage.LanguageTag
        first_page = $firstPage
        render_dir = $(if ($tempDir -ne $null -and $KeepRenders) { $tempDir } else { $null })
    }
    $writer.WriteLine(($meta | ConvertTo-Json -Compress))
    $stage = "recognize pages"
    $total = $pagesToRead.Count
    $done = 0
    foreach ($item in $pagesToRead) {
        if (ShouldCancel) { throw "cancelled" }
        $pageJson = Ocr-File $item.file $item.page
        $writer.WriteLine(($pageJson | ConvertTo-Json -Depth 6 -Compress))
        $writer.Flush()
        $done++
        # stdout carries progress only; the JSON file is the real output.
        Write-Output ("progress {0}/{1}" -f $done, $total)
    }
} finally {
    $writer.Dispose()
    if ($tempDir -ne $null -and -not $KeepRenders) {
        Remove-Item -LiteralPath $tempDir -Recurse -Force -ErrorAction SilentlyContinue
    }
}
} catch {
    # Error messages from WinRT are localized; write them as UTF-8 so the
    # host can report them even when stderr uses a legacy console code page.
    $failedEncoding = [System.Text.UTF8Encoding]::new($false)
    $innermost = $_.Exception.GetBaseException()
    $payload = [pscustomobject]@{ stage = $stage; message = $innermost.Message }
    try {
        [System.IO.File]::WriteAllText(($Out + ".error"), ($payload | ConvertTo-Json -Compress), $failedEncoding)
    } catch {
        # Diagnostics must never mask the original failure.
    }
    throw
}
