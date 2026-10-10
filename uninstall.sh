#!/usr/bin/env sh
# Remove what install.sh put in place.
#   sh uninstall.sh                  -> from ~/.local
#   sh uninstall.sh --purge          -> and your cache, settings and sign-in tokens
#   PREFIX=/usr sh uninstall.sh      -> a system-wide install (run with sudo)
set -eu
PREFIX="${PREFIX:-$HOME/.local}"
APP_ID=io.github.turbinebmw.Skali
BIN="$PREFIX/bin"
SHARE="$PREFIX/share"

purge=false
for arg in "$@"; do
  case $arg in
    --purge) purge=true ;;
    -h|--help) sed -n '2,7p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "unknown option: $arg (try --help)" >&2; exit 2 ;;
  esac
done
is_root=false
if [ "$(id -u)" = 0 ]; then is_root=true; fi
if $purge && $is_root; then
  echo "--purge removes one user's data: run 'sh uninstall.sh --purge' as that user." >&2
  exit 2
fi

# A running Skali (it may be in the background) quits the way Quit does.
if ! $is_root && pgrep -x skali >/dev/null 2>&1; then
  gapplication action "$APP_ID" quit 2>/dev/null || true
fi

# The login unit: disabled, so nothing links to it, then removed.
if [ "$PREFIX" = "$HOME/.local" ]; then UNITDIR="$SHARE/systemd/user"; else UNITDIR="$PREFIX/lib/systemd/user"; fi
if [ -e "$UNITDIR/$APP_ID.service" ]; then
  if ! $is_root; then systemctl --user disable "$APP_ID.service" >/dev/null 2>&1 || true; fi
  rm -f "$UNITDIR/$APP_ID.service"
  if ! $is_root; then systemctl --user daemon-reload >/dev/null 2>&1 || true; fi
fi

# The D-Bus service goes first, so nothing can start Skali again mid-way.
rm -f "$SHARE/dbus-1/services/$APP_ID.service"
rm -f "$BIN/skali"
rm -f "$SHARE/applications/$APP_ID.desktop"
rm -f "$SHARE/metainfo/$APP_ID.metainfo.xml"
for size in 48 64 128 256 512; do
  rm -f "$SHARE/icons/hicolor/${size}x${size}/apps/$APP_ID.png"
done
rm -f "$SHARE/icons/hicolor/symbolic/apps/$APP_ID-symbolic.svg"
for mo in "$SHARE"/locale/*/LC_MESSAGES/skali.mo; do
  if [ -e "$mo" ]; then rm -f "$mo"; fi
done
schemas="$SHARE/glib-2.0/schemas"
if [ -e "$schemas/$APP_ID.gschema.xml" ]; then
  rm -f "$schemas/$APP_ID.gschema.xml"
  # With no schema left to compile, glib-compile-schemas keeps the old
  # compiled file, which still lists Skali's.
  if ls "$schemas"/*.gschema.xml >/dev/null 2>&1; then
    glib-compile-schemas "$schemas" >/dev/null 2>&1 || true
  else
    rm -f "$schemas/gschemas.compiled"
  fi
fi
gtk-update-icon-cache -q -t -f "$SHARE/icons/hicolor" 2>/dev/null || true
update-desktop-database -q "$SHARE/applications" 2>/dev/null || true

# "Start at Login" left an autostart entry that would now fail at every login.
if ! $is_root; then
  rm -f "${XDG_CONFIG_HOME:-$HOME/.config}/autostart/$APP_ID.desktop"
fi

if $purge; then
  rm -rf "${XDG_DATA_HOME:-$HOME/.local/share}/skali"
  rm -rf "${XDG_CACHE_HOME:-$HOME/.cache}/skali"
  if command -v dconf >/dev/null 2>&1; then
    dconf reset -f /io/github/turbinebmw/Skali/ 2>/dev/null || true
  fi
  # The sign-in tokens in the keyring.
  if command -v secret-tool >/dev/null 2>&1; then
    secret-tool clear xdg:schema "$APP_ID.Token" 2>/dev/null || true
  fi
  echo "Removed Skali's cache, settings and sign-in tokens."
fi

echo "Uninstalled Skali from $PREFIX."
