#!/bin/sh
set -e

echo "[enclave] Setting up iptables NAT rules..."

# Create a dedicated chain for enclave egress
iptables -t nat -N ENCLAVE_PROXY 2>/dev/null || true

# Skip the data-plane proxy's own traffic (UID 1500) to avoid redirect loops
iptables -t nat -A ENCLAVE_PROXY -m owner --uid-owner 1500 -j RETURN

# Skip loopback (DNS is handled directly on 127.0.0.1:53, no redirect needed)
iptables -t nat -A ENCLAVE_PROXY -o lo -j RETURN

# Redirect all outbound TCP through the data-plane TCP proxy
iptables -t nat -A ENCLAVE_PROXY -p tcp -j REDIRECT --to-ports 8080

# Attach chain to TCP OUTPUT only (DNS is on loopback, no UDP rules needed)
iptables -t nat -A OUTPUT -p tcp -j ENCLAVE_PROXY

echo "[enclave] iptables rules applied:"
iptables -t nat -L ENCLAVE_PROXY -n --line-numbers

# Point DNS at the local data-plane proxy (127.0.0.1:53).
# Docker populates /etc/hosts with container names (e.g. control-plane),
# so internal hostnames still resolve without going through DNS.
echo "nameserver 127.0.0.1" > /etc/resolv.conf
echo "[enclave] resolv.conf updated: nameserver 127.0.0.1"

# Start the data-plane proxy as UID 1500 (exempt from iptables redirect above)
echo "[enclave] Starting data-plane proxy..."
su -s /bin/sh dataplane -c "/app/data-plane" &
PROXY_PID=$!
echo "[enclave] data-plane proxy started (pid=$PROXY_PID)"

# Give the proxy a moment to start listening
sleep 1

# Run curl test loop — DNS goes via 127.0.0.1:53 (our proxy), TCP via 8080
echo "[enclave] Starting curl test loop (every 2s)..."
while true; do
    echo "[enclave] --- curl test ---"
    curl -s --max-time 5 http://httpbin.org/ip || echo "[enclave] curl failed"
    echo ""
    sleep 2
done
