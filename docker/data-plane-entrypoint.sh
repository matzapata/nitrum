#!/bin/sh
set -e

echo "[nitrum] Setting up iptables NAT rules..."

iptables -t nat -N ENCLAVE_PROXY 2>/dev/null || true
iptables -t nat -A ENCLAVE_PROXY -m owner --uid-owner 1500 -j RETURN
iptables -t nat -A ENCLAVE_PROXY -o lo -j RETURN
iptables -t nat -A ENCLAVE_PROXY -p tcp -j REDIRECT --to-ports 8080
iptables -t nat -A OUTPUT -p tcp -j ENCLAVE_PROXY

echo "[nitrum] iptables rules applied:"
iptables -t nat -L ENCLAVE_PROXY -n --line-numbers

echo "nameserver 127.0.0.1" > /etc/resolv.conf
echo "[nitrum] resolv.conf updated: nameserver 127.0.0.1"

echo "[nitrum] Starting data-plane proxy..."
su -s /bin/sh dataplane -c "/app/data-plane" &
PROXY_PID=$!
echo "[nitrum] data-plane proxy started (pid=$PROXY_PID)"

sleep 1

# Run the customer application passed as Docker CMD
if [ $# -gt 0 ]; then
    echo "[nitrum] Starting customer app: $*"
    "$@" &
    APP_PID=$!
    echo "[nitrum] customer app started (pid=$APP_PID)"
fi

wait
