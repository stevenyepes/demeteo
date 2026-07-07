# Throwaway loopback sshd for the ExecutionPort C2.2 conformance gate
# (docs/EXECUTION_CONSISTENCY_PLAN.md). Password auth, one unprivileged
# user, and bash + git on PATH so the login-shell leg of `exec_contract`
# exercises the same profile-sourcing path a real remote uses.
#
# This image is NOT hardened and NOT for anything but a CI/loopback test:
# it ships a known password and password auth enabled on purpose.
FROM debian:bookworm-slim

RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      openssh-server bash git ca-certificates \
 && rm -rf /var/lib/apt/lists/* \
 && mkdir -p /run/sshd

ARG SSH_USER=demeteo
ARG SSH_PASSWORD=conformance
RUN useradd --create-home --shell /bin/bash "$SSH_USER" \
 && echo "$SSH_USER:$SSH_PASSWORD" | chpasswd \
 && mkdir -p "/home/$SSH_USER/conformance" \
 && chown -R "$SSH_USER:$SSH_USER" "/home/$SSH_USER/conformance"

# Password auth on (throwaway), root login off.
RUN sed -i 's/^#\?PasswordAuthentication.*/PasswordAuthentication yes/' /etc/ssh/sshd_config \
 && sed -i 's/^#\?PermitRootLogin.*/PermitRootLogin no/' /etc/ssh/sshd_config \
 && sed -i 's/^#\?UsePAM.*/UsePAM no/' /etc/ssh/sshd_config

EXPOSE 22
# -D: stay in foreground; -e: log to stderr so `docker logs` shows auth issues.
CMD ["/usr/sbin/sshd", "-D", "-e"]
