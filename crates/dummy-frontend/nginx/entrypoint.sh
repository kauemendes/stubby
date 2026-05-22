#!/bin/sh
# stubby-dummy-frontend entrypoint
#
# Substitutes STUBBY_APP_NAME into the HTML template, HTML-escaping the
# value so it parses the same as the Rust `render_index` path (see
# crates/dummy-frontend/src/lib.rs). The escape set matches that function:
# &  <  >  "  '
set -eu

APP="${STUBBY_APP_NAME:-stubby}"

escape_html() {
    printf '%s' "$1" | sed \
        -e 's/&/\&amp;/g' \
        -e 's/</\&lt;/g' \
        -e 's/>/\&gt;/g' \
        -e 's/"/\&quot;/g' \
        -e "s/'/\&#x27;/g"
}

ESCAPED=$(escape_html "$APP")

# Use a sed delimiter unlikely to collide with HTML/URL content.
sed "s|{{APP_NAME}}|$ESCAPED|g" /etc/stubby/index.html.tmpl > /usr/share/nginx/html/index.html
cp /etc/stubby/style.css /usr/share/nginx/html/style.css

exec nginx -g 'daemon off;'
