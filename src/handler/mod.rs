use anyhow::{bail, Error, Result};
use std::fs::OpenOptions;
use std::io::Write;
use std::process::Command;
use std::str;

pub fn handle_reboot(force: bool) -> Result<(), Error> {
    if !force {
        match get_boot_counter() {
            Some(t) if t <= 0 => bail!("countdown ended, check greenboot-rollback status"),
            None => bail!("boot_counter is not set"),
            _ => {}
        }
    }
    log::info!("restarting system");
    Command::new("systemctl").arg("reboot").status()?;
    Ok(())
}

pub fn handle_rollback() -> Result<(), Error> {
    match get_boot_counter() {
        Some(t) if t <= 0 => {
            log::info!("Greenboot will now attempt rollback");
            let status = Command::new("rpm-ostree").arg("rollback").status()?;
            if status.success() {
                return Ok(());
            }
            bail!(status.to_string());
        }
        _ => bail!("boot_counter is either unset or not equal to 0"),
    }
}

pub fn set_boot_counter(reboot_count: i32) -> Result<()> {
    match get_boot_counter() {
        Some(counter) => {
            log::info!("boot_counter={counter}");
            Ok(())
        }
        None => {
            Command::new("grub2-editenv")
                .arg("-")
                .arg("set")
                .arg(format!("boot_counter={reboot_count}"))
                .status()?;
            log::info!("boot_counter={reboot_count}");
            Ok(())
        }
    }
}

pub fn unset_boot_counter() -> Result<()> {
    Command::new("grub2-editenv")
        .arg("-")
        .arg("unset")
        .arg("boot_counter")
        .status()?;
    Ok(())
}

pub fn handle_boot_success(success: bool) -> Result<()> {
    if success {
        Command::new("grub2-editenv")
            .arg("-")
            .arg("set")
            .arg("boot_success=1")
            .status()?;
        Command::new("grub2-editenv")
            .arg("-")
            .arg("unset")
            .arg("boot_counter")
            .status()?;
    } else {
        Command::new("grub2-editenv")
            .arg("-")
            .arg("set")
            .arg("boot_success=0")
            .status()?;
    }
    Ok(())
}

pub fn handle_motd(state: &str) -> Result<(), Error> {
    let motd = format!("Greenboot {state}.");

    let mut motd_file = OpenOptions::new()
        .create(true)
        .write(true)
        .open("/etc/motd.d/boot-status")?;
    motd_file.write_all(motd.as_bytes())?;
    Ok(())
}

pub fn get_boot_counter() -> Option<i32> {
    let grub_vars = Command::new("grub2-editenv").arg("-").arg("list").output();
    if grub_vars.is_err() {
        return None;
    }
    let grub_vars = grub_vars.unwrap();
    let grub_vars = match str::from_utf8(&grub_vars.stdout[..]) {
        Ok(vars) => vars.split('\n'),
        Err(_) => {
            log::error!("Unable to fetch grub variables");
            return None;
        }
    };

    for var in grub_vars {
        if var.contains("boot_counter") {
            let boot_counter = var.split('=').last();

            match boot_counter.unwrap().parse::<i32>() {
                Ok(count) => return Some(count),
                Err(_) => {
                    log::error!("boot_counter not a valid integer");
                    return None;
                }
            }
        }
    }
    None
}
