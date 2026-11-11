# Builds the contract with every doc comment stripped from the wasm spec.
# The sources are copied to target\nodoc, doc comments (/// and //!) are removed
# there and the copy is built in its own target directory (sharing the regular
# one mixes up the cargo fingerprints of the two builds), so the working tree is
# never touched; target\nodoc is removed afterwards. The wasm lands at the
# default path (target\wasm32v1-none\release).
# Usage: powershell -ExecutionPolicy Bypass -File scripts\build-nodoc.ps1
$ErrorActionPreference = 'Stop'

$root = Split-Path -Parent $PSScriptRoot
$work = Join-Path $root 'target\nodoc'

if (Test-Path $work) { Remove-Item -Recurse -Force $work }
New-Item -ItemType Directory -Force $work | Out-Null
try {
    Copy-Item "$root\Cargo.toml", "$root\Cargo.lock", "$root\build.rs" $work
    Copy-Item -Recurse "$root\src" "$work\src"

    $utf8 = New-Object System.Text.UTF8Encoding $false
    Get-ChildItem -Recurse -Filter '*.rs' "$work\src" | ForEach-Object {
        $text = [IO.File]::ReadAllText($_.FullName)
        $text = $text -replace '(?m)^[ \t]*//[/!].*(\r?\n|$)', ''
        [IO.File]::WriteAllText($_.FullName, $text, $utf8)
    }

    stellar contract build --optimize --manifest-path "$work\Cargo.toml" `
        --out-dir "$root\target\wasm32v1-none\release"
    $code = $LASTEXITCODE
} finally {
    Remove-Item -Recurse -Force $work
}
exit $code
