$ErrorActionPreference = "Stop"

$projectRoot = Split-Path -Parent $PSScriptRoot
Set-Location $projectRoot

$cargo = Get-Command cargo.exe -ErrorAction SilentlyContinue
if (-not $cargo) {
    $cargoPath = Join-Path $HOME ".cargo\bin\cargo.exe"
    if (Test-Path $cargoPath) {
        $cargo = Get-Item $cargoPath
    } else {
        throw "Cargo nao encontrado. Instale o Rust para Windows antes de gerar o instalador."
    }
}
$cargoExe = $cargo.Source
if (-not $cargoExe) {
    $cargoExe = $cargo.FullName
}

$compiler = Get-Command ISCC.exe -ErrorAction SilentlyContinue
if (-not $compiler) {
    $candidates = @(
        "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe",
        "$env:ProgramFiles\Inno Setup 6\ISCC.exe",
        "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe",
        "${env:ProgramFiles(x86)}\Inno Setup 7\ISCC.exe",
        "$env:ProgramFiles\Inno Setup 7\ISCC.exe"
    )
    $compilerPath = $candidates | Where-Object { Test-Path $_ } | Select-Object -First 1
    if (-not $compilerPath) {
        throw "Inno Setup nao encontrado. Instale-o e rode este script novamente."
    }
    $compiler = Get-Item $compilerPath
}
$compilerExe = $compiler.Source
if (-not $compilerExe) {
    $compilerExe = $compiler.FullName
}

$version = (Select-String -Path (Join-Path $projectRoot "Cargo.toml") -Pattern '^version = "([^"]+)"$' |
    Select-Object -First 1).Matches.Groups[1].Value
if (-not $version) {
    throw "Nao consegui ler a versao do Cargo.toml."
}

$vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
$vsDevCmd = $null
if (Test-Path $vswhere) {
    $vsDevCmd = & $vswhere -latest -products "*" -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -find "Common7\Tools\VsDevCmd.bat" |
        Select-Object -First 1
}
if (-not $vsDevCmd -or -not (Test-Path $vsDevCmd)) {
    throw "Ferramentas C++ do Visual Studio nao encontradas. Instale o workload Desenvolvimento para desktop com C++."
}

$buildTarget = Join-Path $projectRoot "target\windows-installer-build"
$buildCommand = 'call "' + $vsDevCmd + '" -arch=x64 -host_arch=x64 && "' + $cargoExe + '" build --release --locked --target-dir "' + $buildTarget + '"'
& $env:ComSpec /d /c $buildCommand
if ($LASTEXITCODE -ne 0) {
    throw "A compilacao do kvm-switch falhou (codigo $LASTEXITCODE)."
}

$env:KVM_SWITCH_VERSION = $version
& $compilerExe "/DAppVersion=$version" (Join-Path $PSScriptRoot "installer.iss")
if ($LASTEXITCODE -ne 0) {
    throw "A geracao do instalador falhou (codigo $LASTEXITCODE)."
}

$installer = Join-Path $projectRoot "target\release\KVM-Switch-Setup-$version.exe"
Write-Output "Instalador: $installer"
