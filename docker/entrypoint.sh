#!/bin/sh
# CraftCloud entrypoint: runs OxiCloud (upstream binary, via upstream's entrypoint) on 127.0.0.1:8087,
# craftcloud-admin on 127.0.0.1:8088, and Caddy on $CRAFTCLOUD_PORT in front of both. If any process
# exits, the others are stopped and the container exits with that status, so Docker's restart policy
# takes over.
set -eu

: "${CRAFTCLOUD_PORT:=8086}"
# OxiCloud's own apps are off by default. Comma-separated list to turn some back on:
#   music, collab (Markdown editor), dav (CalDAV/CardDAV), or all.
: "${CRAFTCLOUD_NATIVE_APPS:=}"

native() {
    case ",${CRAFTCLOUD_NATIVE_APPS}," in
        *",$1,"* | *",all,"*) return 0 ;;
        *) return 1 ;;
    esac
}

# OxiCloud listens only inside the container; Caddy is the public port.
export OXICLOUD_SERVER_HOST=127.0.0.1
export OXICLOUD_SERVER_PORT=8087

# The URL people use to reach CraftCloud (share links, WOPI). OxiCloud would otherwise default to its
# internal address. An explicit OXICLOUD_BASE_URL still wins.
: "${CRAFTCLOUD_PUBLIC_URL:=http://localhost:${CRAFTCLOUD_PORT}}"
: "${OXICLOUD_BASE_URL:=${CRAFTCLOUD_PUBLIC_URL%/}}"
export CRAFTCLOUD_PUBLIC_URL OXICLOUD_BASE_URL

# Sign-in cookies are marked Secure when people reach CraftCloud over https, so the browser sends them
# only over https. An explicit OXICLOUD_COOKIE_SECURE (true or false) still wins.
case "$CRAFTCLOUD_PUBLIC_URL" in
    https://*) : "${OXICLOUD_COOKIE_SECURE:=true}" ;;
    *) : "${OXICLOUD_COOKIE_SECURE:=false}" ;;
esac
export OXICLOUD_COOKIE_SECURE

# Every request reaches OxiCloud from Caddy on 127.0.0.1. Without trusting that hop, OxiCloud's per-IP
# limits (10 sign-ins a minute, 20 token refreshes) counted the whole server as one client. Caddy
# replaces X-Forwarded-For from untrusted clients, so only the real client address arrives.
: "${OXICLOUD_TRUST_PROXY_CIDR:=127.0.0.1/32,::1/128}"
export OXICLOUD_TRUST_PROXY_CIDR

if native music; then export OXICLOUD_ENABLE_MUSIC=true; else export OXICLOUD_ENABLE_MUSIC=false; fi
if native collab; then export OXICLOUD_ENABLE_MARKDOWN_COLLAB=true; else export OXICLOUD_ENABLE_MARKDOWN_COLLAB=false; fi
blocked=""
if ! native dav; then
    blocked="/caldav /caldav/* /carddav /carddav/* /.well-known/caldav /.well-known/carddav"
fi

# Craft web apps in the browser. The setting lives in one file on the data volume, changed from
# Admin › Server; CRAFTCLOUD_WEB_APPS only seeds it on the very first start. craftcloud-admin
# decides per request which /apps/<id>/ may be served (docker/Caddyfile, forward_auth).
: "${CRAFTCLOUD_WEB_APPS:=off}"
case "$CRAFTCLOUD_WEB_APPS" in
    on | true | 1 | yes) seed=on ;;
    off | false | 0 | no | "") seed=off ;;
    *) echo "craftcloud: CRAFTCLOUD_WEB_APPS must be on or off, got '$CRAFTCLOUD_WEB_APPS'" >&2; exit 64 ;;
esac
export CRAFTCLOUD_STATE_DIR=/app/storage/.craftcloud
if [ ! -f "$CRAFTCLOUD_STATE_DIR/settings.env" ]; then
    mkdir -p "$CRAFTCLOUD_STATE_DIR"
    printf '# CraftCloud server settings, written by Admin › Server. Edit only while the server is stopped.\nCRAFTCLOUD_WEB_APPS=%s\nCRAFTCLOUD_DISABLED_APPS=\n' "$seed" > "$CRAFTCLOUD_STATE_DIR/settings.env"
fi
if [ "$(id -u)" -eq 0 ]; then
    chown -R oxicloud:oxicloud "$CRAFTCLOUD_STATE_DIR"
fi
if [ "$seed" = on ] && [ ! -f /srv/craftcloud/craftcloud/apps.json ]; then
    echo "craftcloud: CRAFTCLOUD_WEB_APPS=on, but this image has no web apps (use the -apps image)" >&2
fi
export CRAFTCLOUD_BLOCKED_PATHS="${blocked:-/__craftcloud_nothing_blocked__}"
export CRAFTCLOUD_PORT CRAFTCLOUD_VERSION="${CRAFTCLOUD_VERSION:-dev}"

# Caddy keeps nothing on disk (no TLS, no admin API); point its dirs at /tmp anyway.
export XDG_DATA_HOME=/tmp/caddy-data XDG_CONFIG_HOME=/tmp/caddy-config

/usr/local/bin/entrypoint.sh oxicloud &
oxi_pid=$!

as_oxicloud() {
    if [ "$(id -u)" -eq 0 ]; then su-exec oxicloud "$@"; else "$@"; fi
}

as_oxicloud craftcloud-admin &
admin_pid=$!

as_oxicloud caddy run --config /etc/craftcloud/Caddyfile --adapter caddyfile &
caddy_pid=$!

stop() {
    kill -TERM "$oxi_pid" "$caddy_pid" "$admin_pid" 2>/dev/null || true
}
trap 'stop' TERM INT

# Wait for the first process to exit (or for a signal), then stop the others.
status=0
wait -n "$oxi_pid" "$caddy_pid" "$admin_pid" || status=$?
stop
wait "$oxi_pid" 2>/dev/null || true
wait "$caddy_pid" 2>/dev/null || true
wait "$admin_pid" 2>/dev/null || true
exit "$status"
