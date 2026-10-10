#!/bin/sh
# Regenerate po/campfe.pot: every string marked for translation in the Rust
# sources (gettext, plural), the Blueprints (_ and C_), the desktop file and
# the metainfo. A translation is po/<lang>.po, started with
#   msginit -i po/campfe.pot -o po/<lang>.po -l <lang>
# and refreshed after this with
#   msgmerge -U po/<lang>.po po/campfe.pot
# install.sh compiles them.
set -eu
cd "$(dirname "$0")/.."
out=po/campfe.pot
common="--from-code=UTF-8 --add-comments=TRANSLATORS --package-name=Campfe --msgid-bugs-address=https://github.com/turbineBMW/Campfe/issues"
git ls-files 'crates/*.rs' > po/.rust-files
xgettext $common --language=Rust --keyword=gettext --keyword=plural:1,2 \
  --files-from=po/.rust-files -o "$out"
git ls-files 'crates/campfe/ui/*.blp' > po/.blp-files
xgettext $common --language=C --keyword=_ --keyword=C_:1c,2 \
  --files-from=po/.blp-files --join-existing -o "$out"
xgettext $common --join-existing -o "$out" \
  data/io.github.turbinebmw.Campfe.desktop.in data/io.github.turbinebmw.Campfe.metainfo.xml.in
rm -f po/.rust-files po/.blp-files
echo "Wrote $out ($(grep -c '^msgid ' "$out") strings)"
