$ErrorActionPreference = 'Stop'
$Root = Split-Path -Parent $PSScriptRoot
Set-Location $Root

cargo test -p lunar-shared
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

cargo build --workspace --release --target x86_64-pc-windows-msvc
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

$Output = Join-Path $Root 'dist'
New-Item -ItemType Directory -Force -Path $Output | Out-Null
Copy-Item 'target\x86_64-pc-windows-msvc\release\LunarInjector.exe' $Output
Copy-Item 'target\x86_64-pc-windows-msvc\release\lunar_bridge.dll' $Output
Write-Host "Built: $Output"

$IsccPaths = @(
	"${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe",
	"${env:ProgramFiles}\Inno Setup 6\ISCC.exe",
	"ISCC.exe"
)
$Iscc = $IsccPaths | Where-Object { Test-Path $_ -PathType Leaf } | Select-Object -First 1

if ($Iscc) {
	Write-Host "Compiling Inno Setup installer..."
	& $Iscc "installer.iss"
	if ($LASTEXITCODE -eq 0) {
		Write-Host "Installer created: dist\LunarInjector_Setup.exe"
	} else {
		Write-Warning "ISCC failed with exit code $LASTEXITCODE"
	}
} else {
	Write-Warning "ISCC.exe not found. Install Inno Setup 6 to compile installer.iss"
}
