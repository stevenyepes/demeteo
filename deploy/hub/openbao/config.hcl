# The listener is plain HTTP on purpose: 8200 is reachable only on the compose
# network (no `ports:` on this service, and Caddy never proxies to it), so TLS
# here would add a certificate to rotate without narrowing who can connect.
# Revisit before publishing 8200 or splitting the services across hosts.
#
# mlock stays on (compose grants IPC_LOCK) so Transit keys are never swapped
# to disk in plaintext.
ui = false

storage "file" {
  path = "/openbao/file"
}

listener "tcp" {
  address     = "0.0.0.0:8200"
  tls_disable = true
}
