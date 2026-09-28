#!/bin/sh
# Install, upgrade, or uninstall the a16p binary from GitHub Releases.
#
#   curl -fsSL https://raw.githubusercontent.com/migmolrod/a16p-gen/master/install.sh | sh
#   ... | sh -s -- --init-config        also write the default config if missing
#   ... | sh -s -- --version v0.2.0     install a specific release
#   ... | sh -s -- --bin-dir ~/bin      install somewhere other than ~/.local/bin
#   ... | sh -s -- --uninstall          remove the binary (config is kept)
#
# Rerunning installs the latest release over the existing binary (= upgrade).
set -eu

REPO="migmolrod/a16p-gen"
TARGET="x86_64-unknown-linux-musl"
ASSET="a16p-$TARGET.tar.gz"

version="${A16P_VERSION:-latest}"
bin_dir="${A16P_BIN_DIR:-${XDG_BIN_HOME:-$HOME/.local/bin}}"
init_config=0
uninstall=0

say() { printf 'a16p-install: %s\n' "$*"; }
die() { printf 'a16p-install: error: %s\n' "$*" >&2; exit 1; }

usage() {
    sed -n '2,10s/^# \{0,1\}//p' "$0" 2>/dev/null || true
}

while [ $# -gt 0 ]; do
    case "$1" in
        --version) [ $# -ge 2 ] || die "--version needs a value"; version="$2"; shift 2 ;;
        --bin-dir) [ $# -ge 2 ] || die "--bin-dir needs a value"; bin_dir="$2"; shift 2 ;;
        --init-config) init_config=1; shift ;;
        --uninstall) uninstall=1; shift ;;
        -h | --help) usage; exit 0 ;;
        *) die "unknown argument: $1" ;;
    esac
done

bin="$bin_dir/a16p"

if [ "$uninstall" -eq 1 ]; then
    if [ ! -e "$bin" ]; then
        say "nothing to remove at $bin"
        exit 0
    fi
    config_path="$("$bin" config path 2>/dev/null || true)"
    rm -f "$bin"
    say "removed $bin"
    if [ -n "$config_path" ] && [ -e "$config_path" ]; then
        say "kept config at $config_path (delete it by hand if you no longer want it)"
    fi
    exit 0
fi

os="$(uname -s)"
arch="$(uname -m)"
if [ "$os" != "Linux" ] || [ "$arch" != "x86_64" ]; then
    die "no prebuilt binary for $os/$arch; build from source with:
    cargo install --locked --git https://github.com/$REPO"
fi

# A16P_BASE_URL is for testing against a local server serving dist/.
if [ -n "${A16P_BASE_URL:-}" ]; then
    base="$A16P_BASE_URL"
elif [ "$version" = "latest" ]; then
    base="https://github.com/$REPO/releases/latest/download"
else
    case "$version" in v*) ;; *) version="v$version" ;; esac
    base="https://github.com/$REPO/releases/download/$version"
fi

if command -v curl >/dev/null 2>&1; then
    fetch() { curl -fsSL -o "$2" "$1"; }
elif command -v wget >/dev/null 2>&1; then
    fetch() { wget -q -O "$2" "$1"; }
else
    die "need curl or wget"
fi
command -v sha256sum >/dev/null 2>&1 || die "need sha256sum"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT INT TERM

say "downloading $base/$ASSET"
fetch "$base/$ASSET" "$tmp/$ASSET" || die "download failed: $base/$ASSET"
fetch "$base/$ASSET.sha256" "$tmp/$ASSET.sha256" || die "download failed: $base/$ASSET.sha256"
(cd "$tmp" && sha256sum -c --quiet "$ASSET.sha256") || die "checksum mismatch for $ASSET"
tar -xzf "$tmp/$ASSET" -C "$tmp"

old_version=""
if [ -x "$bin" ]; then
    old_version="$("$bin" --version 2>/dev/null || true)"
fi

mkdir -p "$bin_dir"
# Stage next to the target, then rename: mv within one filesystem is an
# atomic rename, so the old binary -- possibly the one running
# `a16p self-update` right now -- is swapped out, never overwritten.
install -m 755 "$tmp/a16p-$TARGET/a16p" "$bin.new"
mv -f "$bin.new" "$bin"
new_version="$("$bin" --version)"

if [ -n "$old_version" ] && [ "$old_version" != "$new_version" ]; then
    say "upgraded $old_version -> $new_version at $bin"
elif [ -n "$old_version" ]; then
    say "reinstalled $new_version at $bin"
else
    say "installed $new_version at $bin"
fi

case ":$PATH:" in
    *":$bin_dir:"*) ;;
    *) say "warning: $bin_dir is not on your PATH; add it to use 'a16p' directly" ;;
esac

if [ "$init_config" -eq 1 ]; then
    config_path="$("$bin" config path)"
    if [ -e "$config_path" ]; then
        say "config already exists at $config_path, leaving it alone"
    else
        "$bin" config init
    fi
fi
