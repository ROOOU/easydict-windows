Add-Type -AssemblyName System.Drawing
$bmp = New-Object System.Drawing.Bitmap(512, 512)
$graphics = [System.Drawing.Graphics]::FromImage($bmp)
$graphics.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::AntiAlias
$graphics.TextRenderingHint = [System.Drawing.Text.TextRenderingHint]::AntiAliasGridFit

# Background gradient
$brush1 = New-Object System.Drawing.Drawing2D.LinearGradientBrush(
    (New-Object System.Drawing.Point(0, 0)),
    (New-Object System.Drawing.Point(512, 512)),
    [System.Drawing.Color]::FromArgb(99, 102, 241),
    [System.Drawing.Color]::FromArgb(139, 92, 246)
)
$graphics.FillRectangle($brush1, 0, 0, 512, 512)

# Letter T
$font = New-Object System.Drawing.Font('Segoe UI', 240, [System.Drawing.FontStyle]::Bold)
$whiteBrush = New-Object System.Drawing.SolidBrush([System.Drawing.Color]::White)
$sf = New-Object System.Drawing.StringFormat
$sf.Alignment = [System.Drawing.StringAlignment]::Center
$sf.LineAlignment = [System.Drawing.StringAlignment]::Center
$rect = New-Object System.Drawing.RectangleF(0, 0, 512, 512)
$graphics.DrawString('T', $font, $whiteBrush, $rect, $sf)

$outDir = 'c:\Users\12064\Desktop\translate\src-tauri\icons'

# Save multiple sizes
$bmp.Save("$outDir\icon.png", [System.Drawing.Imaging.ImageFormat]::Png)

# 32x32
$bmp32 = New-Object System.Drawing.Bitmap($bmp, 32, 32)
$bmp32.Save("$outDir\32x32.png", [System.Drawing.Imaging.ImageFormat]::Png)
$bmp32.Dispose()

# 128x128
$bmp128 = New-Object System.Drawing.Bitmap($bmp, 128, 128)
$bmp128.Save("$outDir\128x128.png", [System.Drawing.Imaging.ImageFormat]::Png)
$bmp128.Dispose()

# 256x256 (for 128x128@2x)
$bmp256 = New-Object System.Drawing.Bitmap($bmp, 256, 256)
$bmp256.Save("$outDir\128x128@2x.png", [System.Drawing.Imaging.ImageFormat]::Png)
$bmp256.Dispose()

# ICO file (using 256x256)
$bmpIco = New-Object System.Drawing.Bitmap($bmp, 256, 256)
$icon = [System.Drawing.Icon]::FromHandle($bmpIco.GetHicon())
$fs = New-Object System.IO.FileStream("$outDir\icon.ico", [System.IO.FileMode]::Create)
$icon.Save($fs)
$fs.Close()
$icon.Dispose()
$bmpIco.Dispose()

$bmp.Dispose()
$graphics.Dispose()
Write-Host 'Icons created successfully'
