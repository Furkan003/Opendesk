param([string]$AndroidSdk = "$env:LOCALAPPDATA/Android/Sdk")
$ErrorActionPreference = 'Stop'
$opendeskProject = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$opendeskNdk = Get-ChildItem -LiteralPath (Join-Path $AndroidSdk 'ndk') -Directory | Sort-Object { [version]$_.Name } -Descending | Select-Object -First 1
if ($null -eq $opendeskNdk) {throw 'Install an Android NDK through Android Studio first'}
$opendeskToolchain = Join-Path $opendeskNdk.FullName 'toolchains/llvm/prebuilt/windows-x86_64/bin'
$env:CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER = Join-Path $opendeskToolchain 'aarch64-linux-android24-clang.cmd'
$env:CC_aarch64_linux_android = $env:CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER
$env:AR_aarch64_linux_android = Join-Path $opendeskToolchain 'llvm-ar.exe'
rustup target add aarch64-linux-android
if ($LASTEXITCODE -ne 0) {throw 'Rust Android target installation failed'}
$opendeskTarget = Join-Path $opendeskProject 'work/android-target'
cargo build --manifest-path (Join-Path $opendeskProject 'Cargo.toml') --no-default-features --lib --target aarch64-linux-android --release --target-dir $opendeskTarget
if ($LASTEXITCODE -ne 0) {throw 'Rust core compilation failed'}
$opendeskJni = Join-Path $opendeskProject 'android-app/android/app/src/main/jniLibs/arm64-v8a'
New-Item -ItemType Directory -Path $opendeskJni -Force | Out-Null
Copy-Item -LiteralPath (Join-Path $opendeskTarget 'aarch64-linux-android/release/libopendesk.so') -Destination (Join-Path $opendeskJni 'libopendesk.so')
Push-Location (Join-Path $opendeskProject 'android-app')
try {
    flutter pub get
    if ($LASTEXITCODE -ne 0) {throw 'Flutter dependency resolution failed'}
    flutter build apk --release --target-platform android-arm64
    if ($LASTEXITCODE -ne 0) {throw 'Android APK build failed'}
} finally {Pop-Location}
