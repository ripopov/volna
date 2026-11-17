#!/usr/bin/env bash
set -eo pipefail

root=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
prefix=${CARGO_INSTALL_ROOT:-${CARGO_HOME:-"$HOME/.cargo"}}
uninstall=false
cargo_args=()
while (($#)); do
	case "$1" in
		--root)
			[[ $# -ge 2 && -n $2 ]] || { echo "--root requires a directory" >&2; exit 2; }
			prefix=$2; shift 2 ;;
		--root=*) prefix=${1#*=}; shift ;;
		--uninstall) uninstall=true; shift ;;
		-h|--help)
			printf 'Usage: %s [--root DIR] [--uninstall] [Cargo options]\n' "$0"
			exit 0 ;;
		*) cargo_args+=("$1"); shift ;;
	esac
done

mkdir -p -- "$prefix"
prefix=$(CDPATH= cd -- "$prefix" && pwd)
cd -- "$root"
if $uninstall; then
	cargo uninstall --root "$prefix" volna vtr-cli "${cargo_args[@]}"
else
	for package in vtr/vtr-cli volna; do
		cargo install --path "$root/$package" --root "$prefix" --locked --force "${cargo_args[@]}"
	done
fi

[[ $(uname -s) == Linux ]] || exit 0
data=${XDG_DATA_HOME:-"$HOME/.local/share"}
launcher=$data/applications/io.github.ripopov.volna.desktop
themes=$data/icons/hicolor
sizes="16 24 32 48 64 128 256 512"
executable=$prefix/bin/volna
try_exec=${executable//\\/\\\\}
if $uninstall; then
	grep -Fxq -- "TryExec=$try_exec" "$launcher" 2>/dev/null || exit 0
	rm -f -- "$launcher"
	for size in $sizes; do
		rm -f -- "$themes/${size}x${size}/apps/io.github.ripopov.volna.png"
	done
else
	exec_path=$(printf '%s' "$executable" | sed 's/[\\"$`]/\\&/g; s/\\/\\\\/g; s/%/%%/g')
	mkdir -p -- "$data/applications"
	while IFS= read -r line; do
		case "$line" in
			# Keep percent escapes out of the program field: GLib checks it before expansion.
			Exec=*) printf '%s "%s" %%f\n' 'Exec=/bin/sh -c "exec \\"\\$0\\" \\"\\$@\\""' "$exec_path" ;;
			TryExec=*) printf 'TryExec=%s\n' "$try_exec" ;;
			*) printf '%s\n' "$line" ;;
		esac
	done < "$root/volna/packaging/io.github.ripopov.volna.desktop" > "$launcher"
	chmod 644 "$launcher"
	for size in $sizes; do
		install -Dm644 "$root/volna/assets/app-icon/volna-$size.png" \
			"$themes/${size}x${size}/apps/io.github.ripopov.volna.png"
	done
fi

if command -v update-desktop-database >/dev/null 2>&1; then
	update-desktop-database "$data/applications" || true
fi
if [[ -f "$themes/index.theme" ]] && command -v gtk-update-icon-cache >/dev/null 2>&1; then
	gtk-update-icon-cache --force --quiet "$themes" || true
fi
