#!/usr/bin/env bash
# Generate a self-signed TLS certificate for local development of snip-sync.
# DO NOT use these certs in production. Use Let's Encrypt or another CA.
#
# Usage:
#   ./scripts/gen-dev-cert.sh        # writes cert.pem and key.pem in CWD
#   ./scripts/gen-dev-cert.sh ./certs
set -euo pipefail

out_dir="${1:-.}"
mkdir -p "$out_dir"
cert_path="$out_dir/cert.pem"
key_path="$out_dir/key.pem"

if [ -e "$cert_path" ] || [ -e "$key_path" ]; then
    echo "Refusing to overwrite existing certs in $out_dir." >&2
    echo "Delete them first, or pass a different output directory." >&2
    exit 1
fi

echo "Generating self-signed certificate in $out_dir..."
openssl req -x509 -newkey rsa:4096 -nodes \
    -keyout "$key_path" \
    -out "$cert_path" \
    -days 365 \
    -subj "/CN=localhost" \
    -addext "subjectAltName=DNS:localhost,IP:127.0.0.1"

chmod 600 "$key_path"
chmod 644 "$cert_path"

echo "Wrote:"
echo "  cert: $cert_path"
echo "  key:  $key_path (mode 600)"
echo
echo "These are reverse-proxy development assets."
echo "snip-sync does not read TLS_CERT/TLS_KEY and does not terminate TLS itself."
