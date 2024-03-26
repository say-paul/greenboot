FROM quay.io/centos-bootc/centos-bootc:stream9
RUN rpm-ostree install git findutils systemd grub2-tools-minimal util-linux jq 
COPY ./rpmbuild/RPMS/x86_64 /greenboot-rpms
WORKDIR /greenboot-rpms
RUN rpm-ostree install greenboot-0.15.4-1.fc39.x86_64.rpm 
RUN rpm-ostree install greenboot-default-health-checks-0.15.4-1.fc39.x86_64.rpm
RUN systemctl enable greenboot-grub2-set-counter.service
RUN systemctl enable greenboot-grub2-set-success.service
RUN systemctl enable greenboot-healthcheck.service
RUN systemctl enable greenboot-loading-message.service
RUN systemctl enable greenboot-rpm-ostree-grub2-check-fallback.service
RUN systemctl enable redboot-auto-reboot.service
RUN systemctl enable redboot-task-runner.service
RUN systemctl enable redboot.target