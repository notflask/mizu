#!/usr/bin/env bash
# Build mizu from this checkout and install it for the current user.
# Linux and macOS. Runs on bash 3.2 (the macOS default).
#
#   scripts/install.sh [--prefix DIR] [--system] [--uninstall] [--no-build]
#                      [--universal] [--nix | --no-nix] [--default-pdf]
#                      [--dry-run] [-h]

set -euo pipefail

APP_ID="io.github.notflask.Mizu"
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OS="$(uname -s)"

prefix=""
system=0
uninstall=0
build=1
universal=0
nix_mode="auto"
default_pdf=0
dry=0

usage() {
	sed -n '2,8p' "$0" | sed 's/^# \{0,1\}//'
	cat <<'EOF'

  --prefix DIR     install under DIR (default: ~/.local; macOS: ~/Applications + ~/.local/bin)
  --system         install for all users (/usr/local, /Applications), sudo only for copying
  --uninstall      remove what a previous run installed
  --no-build       install the existing target/release/mizu
  --universal      macOS: build arm64 + x86_64 and join them
  --nix / --no-nix force or skip the Nix path (auto on NixOS)
  --default-pdf    also make mizu the default app for PDF and EPUB
  --dry-run        print what would happen
  -h, --help       this text
EOF
}

while [ $# -gt 0 ]; do
	case "$1" in
	--prefix)
		[ $# -ge 2 ] || { echo "--prefix needs a directory" >&2; exit 2; }
		prefix="$2"
		shift
		;;
	--prefix=*) prefix="${1#--prefix=}" ;;
	--system) system=1 ;;
	--uninstall) uninstall=1 ;;
	--no-build) build=0 ;;
	--universal) universal=1 ;;
	--nix) nix_mode="yes" ;;
	--no-nix) nix_mode="no" ;;
	--default-pdf) default_pdf=1 ;;
	--dry-run) dry=1 ;;
	-h | --help)
		usage
		exit 0
		;;
	*)
		echo "unknown option: $1 (see --help)" >&2
		exit 2
		;;
	esac
	shift
done

# ---------------------------------------------------------------------------
# helpers

say() { printf '\033[1m==>\033[0m %s\n' "$*"; }
note() { printf '    %s\n' "$*"; }
warn() { printf '\033[33mwarning:\033[0m %s\n' "$*" >&2; }
die() {
	printf '\033[31merror:\033[0m %s\n' "$*" >&2
	exit 1
}

# Run a command, or only print it with --dry-run.
run() {
	if [ "$dry" = 1 ]; then
		printf '    $ %s\n' "$*"
	else
		"$@"
	fi
}

# Same, with sudo for --system installs.
SUDO=""
srun() {
	if [ -n "$SUDO" ]; then
		run "$SUDO" "$@"
	else
		run "$@"
	fi
}

have() { command -v "$1" >/dev/null 2>&1; }

ask() {
	# ask "question" -> 0 for yes. Defaults to no without a terminal.
	[ -t 0 ] || return 1
	printf '%s [y/N] ' "$1"
	local a
	read -r a || return 1
	case "$a" in y | Y | yes | YES) return 0 ;; *) return 1 ;; esac
}

version() {
	sed -n 's/^version *= *"\(.*\)"/\1/p' "$REPO/Cargo.toml" | head -1
}

on_path() {
	case ":$PATH:" in *":$1:"*) return 0 ;; *) return 1 ;; esac
}

# The manifest lists every installed file, one per line, for --uninstall.
MANIFEST=""
manifest_add() {
	[ "$dry" = 1 ] && return 0
	if [ -n "$SUDO" ]; then
		printf '%s\n' "$1" | $SUDO tee -a "$MANIFEST" >/dev/null
	else
		printf '%s\n' "$1" >>"$MANIFEST"
	fi
}

manifest_start() {
	srun mkdir -p "$(dirname "$MANIFEST")"
	if [ "$dry" = 0 ]; then
		if [ -n "$SUDO" ]; then
			$SUDO sh -c ": > '$MANIFEST'"
		else
			: >"$MANIFEST"
		fi
	fi
	manifest_add "$MANIFEST"
}

do_uninstall() {
	[ -f "$MANIFEST" ] || die "nothing to uninstall (no manifest at $MANIFEST)"
	say "Removing mizu (manifest: $MANIFEST)"
	# Longest paths first, so files go before the directories that hold them.
	local f
	awk '{ print length, $0 }' "$MANIFEST" | sort -rn | cut -d' ' -f2- | while IFS= read -r f; do
		[ -n "$f" ] || continue
		if [ -d "$f" ] && [ ! -L "$f" ]; then
			case "$f" in
			*.app) srun rm -rf "$f" ;;
			# Other directories only go when empty: they may hold user data.
			*) srun rmdir "$f" 2>/dev/null || continue ;;
			esac
		elif [ -e "$f" ] || [ -L "$f" ]; then
			srun rm -f "$f"
		fi
		note "removed $f"
	done
	after_uninstall
	say "Done."
}

# ---------------------------------------------------------------------------
# Linux

is_nixos() {
	[ -e /etc/NIXOS ] && return 0
	[ -r /etc/os-release ] && grep -q '^ID=nixos' /etc/os-release
}

linux_pkg_hint() {
	# Print the install command for the build dependencies on this distro.
	local id="" like=""
	if [ -r /etc/os-release ]; then
		id="$(sed -n 's/^ID=//p' /etc/os-release | tr -d '"')"
		like="$(sed -n 's/^ID_LIKE=//p' /etc/os-release | tr -d '"')"
	fi
	case " $id $like " in
	*" debian "* | *" ubuntu "*)
		echo "sudo apt-get install -y clang libclang-dev pkg-config make libfontconfig1-dev libfreetype6-dev libwayland-dev libxkbcommon-dev libx11-dev libxcursor-dev libxi-dev libxrandr-dev"
		;;
	*" fedora "* | *" rhel "* | *" centos "*)
		echo "sudo dnf install -y clang clang-devel pkgconf-pkg-config make fontconfig-devel freetype-devel wayland-devel libxkbcommon-devel libX11-devel libXcursor-devel libXi-devel libXrandr-devel"
		;;
	*" arch "* | *" archlinux "*)
		echo "sudo pacman -S --needed clang pkgconf make fontconfig freetype2 wayland libxkbcommon libx11 libxcursor libxi libxrandr"
		;;
	*" suse "* | *" opensuse "*)
		echo "sudo zypper install -y clang llvm-devel pkg-config make fontconfig-devel freetype2-devel wayland-devel libxkbcommon-devel libX11-devel libXcursor-devel libXi-devel libXrandr-devel"
		;;
	*) echo "" ;;
	esac
}

linux_check_deps() {
	if ! have cargo; then
		die "cargo is missing. Install Rust from https://rustup.rs and run this again."
	fi
	local missing=""
	have clang || missing="$missing clang"
	have pkg-config || missing="$missing pkg-config"
	have make || missing="$missing make"
	if have pkg-config; then
		local p
		for p in fontconfig freetype2 wayland-client xkbcommon; do
			pkg-config --exists "$p" || missing="$missing $p"
		done
	fi
	[ -z "$missing" ] && return 0
	warn "missing build dependencies:$missing"
	local hint
	hint="$(linux_pkg_hint)"
	if [ -n "$hint" ]; then
		note "install them with:"
		note "  $hint"
		if ask "Run that now?"; then
			run sh -c "$hint"
			return 0
		fi
	else
		note "install them with your package manager (development packages)."
	fi
	die "cannot build without them"
}

linux_install_nix() {
	have nix || die "nix is not available"
	local flake="path:$REPO#mizu"
	if [ "$uninstall" = 1 ]; then
		say "Removing mizu from the Nix profile"
		run nix profile remove mizu || warn "no profile entry named mizu"
		return 0
	fi
	if nix profile list 2>/dev/null | grep -q "^Name: *mizu"; then
		say "Upgrading mizu in your Nix profile"
		# A path: flake is re-read on upgrade; remove and add again to be sure.
		run nix profile remove mizu
	else
		say "Installing mizu into your Nix profile"
	fi
	# `nix profile install` is called `add` since Nix 2.27 or so.
	local add=install
	if nix profile add --help >/dev/null 2>&1; then add=add; fi
	run nix profile "$add" "$flake"
	say "Done. mizu, its desktop entry and icons are in ~/.nix-profile."
	note "For a declarative install (NixOS / home-manager):"
	note "  inputs.mizu.url = \"github:notflask/mizu\";"
	note "  nixpkgs.overlays = [ mizu.overlays.default ];"
	note "  environment.systemPackages = [ pkgs.mizu ];"
	if [ "$default_pdf" = 1 ] && have xdg-mime; then
		run xdg-mime default "$APP_ID.desktop" application/pdf application/epub+zip
	fi
}

linux_refresh_caches() {
	local share="$1"
	if have update-desktop-database; then
		srun update-desktop-database -q "$share/applications" 2>/dev/null || true
	fi
	# Only refresh an icon cache that exists; creating one in ~/.local is
	# not needed and would outlive --uninstall.
	if have gtk-update-icon-cache && [ -f "$share/icons/hicolor/icon-theme.cache" ]; then
		srun gtk-update-icon-cache -q -f -t "$share/icons/hicolor" 2>/dev/null || true
	fi
	if have xdg-desktop-menu; then
		run xdg-desktop-menu forceupdate 2>/dev/null || true
	fi
}

after_uninstall() {
	if [ "$OS" = Linux ]; then
		linux_refresh_caches "$prefix/share"
	fi
}

linux_main() {
	if [ "$nix_mode" = yes ] || { [ "$nix_mode" = auto ] && is_nixos; }; then
		linux_install_nix
		return
	fi
	if is_nixos; then
		warn "a plain cargo build does not find Vulkan/Wayland on NixOS; prefer the default (Nix) path"
	fi

	if [ -z "$prefix" ]; then
		if [ "$system" = 1 ]; then prefix=/usr/local; else prefix="$HOME/.local"; fi
	fi
	MANIFEST="$prefix/share/mizu/install-manifest"
	if [ "$uninstall" = 1 ]; then
		do_uninstall
		return
	fi

	local bin="$REPO/target/release/mizu"
	if [ "$build" = 1 ]; then
		linux_check_deps
		say "Building mizu $(version) (release, this takes a few minutes the first time)"
		run cargo build --release --locked --manifest-path "$REPO/Cargo.toml"
	fi
	[ "$dry" = 1 ] || [ -x "$bin" ] || die "no binary at $bin (drop --no-build)"

	say "Installing into $prefix"
	manifest_start
	local share="$prefix/share"
	srun install -Dm755 "$bin" "$prefix/bin/mizu"
	manifest_add "$prefix/bin/mizu"

	local desktop="$share/applications/$APP_ID.desktop"
	srun mkdir -p "$share/applications"
	if on_path "$prefix/bin"; then
		srun install -m644 "$REPO/packaging/linux/$APP_ID.desktop" "$desktop"
	else
		# Launchers do not see a PATH that only the shell knows about.
		if [ "$dry" = 1 ]; then
			note "write $desktop with Exec=$prefix/bin/mizu"
		else
			sed "s|^Exec=mizu |Exec=$prefix/bin/mizu |" "$REPO/packaging/linux/$APP_ID.desktop" |
				${SUDO:+$SUDO} tee "$desktop" >/dev/null
		fi
	fi
	manifest_add "$desktop"

	srun install -Dm644 "$REPO/packaging/linux/$APP_ID.metainfo.xml" \
		"$share/metainfo/$APP_ID.metainfo.xml"
	manifest_add "$share/metainfo/$APP_ID.metainfo.xml"

	local src="$REPO/assets/icons/generated/linux/hicolor" f rel
	if [ -d "$src" ]; then
		while IFS= read -r f; do
			rel="${f#"$src"/}"
			srun install -Dm644 "$f" "$share/icons/hicolor/$rel"
			manifest_add "$share/icons/hicolor/$rel"
		done < <(find "$src" -type f)
	fi
	manifest_add "$share/mizu"

	linux_refresh_caches "$share"

	if [ "$default_pdf" = 1 ]; then
		if have xdg-mime; then
			run xdg-mime default "$APP_ID.desktop" application/pdf application/epub+zip
		else
			warn "xdg-mime not found; set the default app in your desktop settings"
		fi
	fi

	say "Done. Installed mizu $(version):"
	note "$prefix/bin/mizu"
	note "$desktop"
	if ! on_path "$prefix/bin"; then
		warn "$prefix/bin is not on your PATH. Add it, e.g.:"
		note "  fish:      fish_add_path $prefix/bin"
		note "  bash/zsh:  echo 'export PATH=\"$prefix/bin:\$PATH\"' >> ~/.profile"
	fi
}

# ---------------------------------------------------------------------------
# macOS

LSREGISTER=/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister

mac_build() {
	have cargo || die "cargo is missing. Install Rust from https://rustup.rs and run this again."
	if ! xcode-select -p >/dev/null 2>&1; then
		warn "the Xcode Command Line Tools are missing"
		if ask "Install them now (xcode-select --install)?"; then
			run xcode-select --install
		fi
		die "run this again once the Command Line Tools are installed"
	fi
	say "Building mizu $(version) (release, this takes a few minutes the first time)"
	if [ "$universal" = 1 ]; then
		have rustup || die "--universal needs rustup"
		run rustup target add aarch64-apple-darwin x86_64-apple-darwin
		run cargo build --release --locked --manifest-path "$REPO/Cargo.toml" --target aarch64-apple-darwin
		run cargo build --release --locked --manifest-path "$REPO/Cargo.toml" --target x86_64-apple-darwin
		run mkdir -p "$REPO/target/universal"
		run lipo -create -output "$REPO/target/universal/mizu" \
			"$REPO/target/aarch64-apple-darwin/release/mizu" \
			"$REPO/target/x86_64-apple-darwin/release/mizu"
	else
		run cargo build --release --locked --manifest-path "$REPO/Cargo.toml"
	fi
}

mac_main() {
	local appdir bindir
	if [ "$system" = 1 ]; then
		appdir=/Applications
		bindir=/usr/local/bin
	elif [ -n "$prefix" ]; then
		appdir="$prefix/Applications"
		bindir="$prefix/bin"
	else
		appdir="$HOME/Applications"
		bindir="$HOME/.local/bin"
	fi
	if [ -n "$prefix" ]; then
		MANIFEST="$prefix/share/mizu/install-manifest"
	else
		MANIFEST="$HOME/Library/Application Support/$APP_ID/install-manifest"
	fi
	[ "$system" = 1 ] && MANIFEST="/Library/Application Support/$APP_ID/install-manifest"

	if [ "$uninstall" = 1 ]; then
		do_uninstall
		return
	fi

	[ "$build" = 1 ] && mac_build
	local bin="$REPO/target/release/mizu"
	[ "$universal" = 1 ] && bin="$REPO/target/universal/mizu"
	[ "$dry" = 1 ] || [ -x "$bin" ] || die "no binary at $bin (drop --no-build)"

	local stage="$REPO/target/bundle/mizu.app"
	say "Making mizu.app"
	run rm -rf "$stage"
	run "$REPO/packaging/macos/make-app.sh" "$bin" "$(version)" "$stage"
	run codesign --force --deep --sign - "$stage"

	local app="$appdir/mizu.app"
	if [ -d "$app" ]; then
		local id
		id="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$app/Contents/Info.plist" 2>/dev/null || true)"
		[ "$id" = "$APP_ID" ] || die "$app exists and is not mizu ($id); not touching it"
	fi

	say "Installing $app"
	manifest_start
	srun mkdir -p "$appdir"
	srun rm -rf "$app"
	srun cp -R "$stage" "$app"
	manifest_add "$app"
	srun xattr -dr com.apple.quarantine "$app" 2>/dev/null || true
	if [ -x "$LSREGISTER" ]; then
		run "$LSREGISTER" -f "$app" || true
	fi

	# A wrapper, not a symlink: the process has to start from inside the
	# bundle to get its icon and bundle identity.
	srun mkdir -p "$bindir"
	if [ "$dry" = 1 ]; then
		note "write $bindir/mizu -> $app/Contents/MacOS/mizu"
	else
		printf '#!/bin/sh\nexec "%s/Contents/MacOS/mizu" "$@"\n' "$app" |
			${SUDO:+$SUDO} tee "$bindir/mizu" >/dev/null
		srun chmod 755 "$bindir/mizu"
	fi
	manifest_add "$bindir/mizu"

	if [ "$default_pdf" = 1 ]; then
		if have duti; then
			run duti -s "$APP_ID" com.adobe.pdf viewer
			run duti -s "$APP_ID" org.idpf.epub-container viewer
		else
			warn "duti not found. In Finder: Get Info on a PDF, Open with: mizu, Change All."
		fi
	fi

	say "Done. Installed mizu $(version):"
	note "$app"
	note "$bindir/mizu"
	if ! on_path "$bindir"; then
		warn "$bindir is not on your PATH. Add it, e.g.:"
		note "  echo 'export PATH=\"$bindir:\$PATH\"' >> ~/.zprofile"
	fi
}

# ---------------------------------------------------------------------------

if [ "$system" = 1 ] && [ "$(id -u)" != 0 ]; then
	SUDO=sudo
fi

case "$OS" in
Linux) linux_main ;;
Darwin) mac_main ;;
*) die "unsupported system: $OS (Linux and macOS only; Windows has a zip on the releases page)" ;;
esac
