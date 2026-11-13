#!/bin/sh
# Installe RaspyTV (version Rust) sur le Raspberry Pi, à côté de la version Electron.
# À lancer depuis le dossier qui contient raspytv, raspytv-kiosk et web/ (archive de dist/).
# Rien n'est activé au démarrage : `systemctl --user enable raspytv-rs` reste un choix explicite.
set -eu
HERE="$(cd "$(dirname "$0")" && pwd)"
DEST="$HOME/.local/share/raspytv-rs"
CONF="${XDG_CONFIG_HOME:-$HOME/.config}/raspytv-rs"

echo "Dépendances système (WebKitGTK, GTK layer-shell, son, Bluetooth)…"
sudo apt-get install -y --no-install-recommends \
  libwebkit2gtk-4.1-0 libgtk-layer-shell0 libudev1 pulseaudio-utils bluez playerctl

mkdir -p "$DEST" "$CONF"
chmod 700 "$CONF"
install -m 755 "$HERE/raspytv" "$HERE/raspytv-kiosk" "$DEST/"
rm -rf "$DEST/web"
cp -r "$HERE/web" "$DEST/web"
if [ ! -f "$CONF/site.json" ]; then
  if [ -f "$HOME/.config/raspy-tv/site.json" ]; then
    cp "$HOME/.config/raspy-tv/site.json" "$CONF/site.json"
  else
    cp "$HERE/site.example.json" "$CONF/site.json"
  fi
  chmod 600 "$CONF/site.json"
fi
mkdir -p "$HOME/.config/systemd/user"
cp "$HERE/raspytv-rs.service" "$HOME/.config/systemd/user/"
systemctl --user daemon-reload
echo "RaspyTV (Rust) installé dans $DEST. Réglages : $CONF/site.json"
echo "Essai : systemctl --user start raspytv-rs   (arrêter d'abord raspytv, la version Electron)"
