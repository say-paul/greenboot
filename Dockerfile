FROM quay.io/centos-bootc/centos-bootc:stream9
RUN dnf install -y git findutils systemd grub2-tools-minimal util-linux jq

COPY ./rpmbuild/RPMS/x86_64 /greenboot-rpms
WORKDIR /greenboot-rpms
RUN dnf install -y greenboot-0.15.4-1.fc39.x86_64.rpm greenboot-default-health-checks-0.15.4-1.fc39.x86_64.rpm
RUN systemctl enable greenboot-grub2-set-counter \
    greenboot-grub2-set-success.service greenboot-healthcheck.service \
    greenboot-loading-message.service greenboot-rpm-ostree-grub2-check-fallback.service \
    redboot-auto-reboot.service redboot-task-runner.service redboot.target
