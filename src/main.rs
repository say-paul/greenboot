mod handler;
use anyhow::{bail, Error, Result};
use clap::{Parser, Subcommand, ValueEnum};
use config::{Config, File, FileFormat};
use glob::glob;
use handler::*;
use nix::sys::socket::SockaddrLike;
use serde::Deserialize;
use std::path::Path;
use std::process::Command;
use std::str;
use std::time::{Duration, SystemTime};
use systemctl;

static GREENBOOT_INSTALL_PATHS: [&str; 2] = ["/usr/lib/greenboot", "/etc/greenboot"];
static GREENBOOT_CONFIG_FILE: &str = "/etc/greenboot/greenboot.conf";

#[derive(Parser)]
#[clap(author, version, about, long_about = None)]
#[clap(propagate_version = true)]
struct Cli {
    #[clap(value_enum, short, long, default_value_t = LogLevel::Info)]
    log_level: LogLevel,
    #[clap(subcommand)]
    command: Commands,
}
#[derive(Debug, Deserialize)]
struct GreenbootConfig {
    //max reboot attempts if diagnostics fails
    max_reboot: i32,
}

impl GreenbootConfig {
    fn set_default() -> Self {
        Self { max_reboot: 3 }
    }

    fn get_config() -> Self {
        let mut config = Self::set_default();
        let parsed = Config::builder()
            .add_source(File::new(GREENBOOT_CONFIG_FILE, FileFormat::Ini))
            .build();
        match parsed {
            Ok(c) => {
                config.max_reboot = match c.get_int("GREENBOOT_MAX_BOOT_ATTEMPTS") {
                    Ok(c) => c.try_into().unwrap_or_else(|e| {
                        log::warn!("{e}, using default value");
                        config.max_reboot
                    }),
                    Err(e) => {
                        log::warn!("{e}, using default value");
                        config.max_reboot
                    }
                }
            }
            Err(e) => log::warn!("{e}, using default value"),
        }
        config
    }
}

#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, ValueEnum)]
enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
    Off,
}

impl LogLevel {
    fn to_log(self) -> log::LevelFilter {
        match self {
            LogLevel::Trace => log::LevelFilter::Trace,
            LogLevel::Debug => log::LevelFilter::Debug,
            LogLevel::Info => log::LevelFilter::Info,
            LogLevel::Warn => log::LevelFilter::Warn,
            LogLevel::Error => log::LevelFilter::Error,
            LogLevel::Off => log::LevelFilter::Off,
        }
    }
}

#[derive(Subcommand)]
enum Commands {
    HealthCheck,
    Rollback,
    PocRollback,
    PocServiceMinitor,
}

fn run_diagnostics() -> Result<(), Error> {
    let mut script_failure: bool = false;
    let mut path_exists: bool = false;
    for path in GREENBOOT_INSTALL_PATHS {
        let greenboot_required_path = format!("{path}/check/required.d/");
        if !Path::new(&greenboot_required_path).is_dir() {
            continue;
        }
        path_exists = true;
        let greenboot_required_path = format!("{greenboot_required_path}*.sh");
        for entry in glob(&greenboot_required_path)?.flatten() {
            log::info!("running required check {}", entry.to_string_lossy());
            let output = Command::new("bash").arg("-C").arg(entry.clone()).output()?;
            if !output.status.success() {
                log::error!("required script {} failed!", entry.to_string_lossy());
                log::error!("reason: {}", String::from_utf8_lossy(&output.stderr));
                script_failure = true;
            }
        }
    }

    if !path_exists {
        bail!("required.d not found");
    }

    for path in GREENBOOT_INSTALL_PATHS {
        let gereenboot_wanted_path = format!("{path}/check/wanted.d/*.sh");
        for entry in glob(&gereenboot_wanted_path)?.flatten() {
            log::info!("running wanted check {}", entry.to_string_lossy());
            let output = Command::new("bash").arg("-C").arg(entry.clone()).output()?;
            if !output.status.success() {
                // combine and print stderr/stdout
                log::warn!("wanted script {} failed!", entry.to_string_lossy());
                log::warn!("reason: {}", String::from_utf8_lossy(&output.stderr));
            }
        }
    }

    if script_failure {
        bail!("health-check failed!");
    }
    Ok(())
}

fn run_red() -> Result<(), Error> {
    for path in GREENBOOT_INSTALL_PATHS {
        let red_path = format!("{path}/red.d/*.*");
        for entry in glob(&red_path)?.flatten() {
            log::info!("running red check {}", entry.to_string_lossy());
            let output = Command::new("bash").arg("-C").arg(entry.clone()).output()?;
            if !output.status.success() {
                // combine and print stderr/stdout
                log::warn!("red script: {} failed!", entry.to_string_lossy());
                log::warn!("reason: {}", String::from_utf8_lossy(&output.stderr));
            }
        }
    }
    Ok(())
}

fn run_green() -> Result<(), Error> {
    for path in GREENBOOT_INSTALL_PATHS {
        let green_path = format!("{path}/green.d/*.*");
        for entry in glob(&green_path)?.flatten() {
            log::info!("running green check {}", entry.to_string_lossy());
            let output = Command::new("bash").arg("-C").arg(entry.clone()).output()?;
            if !output.status.success() {
                // combine and print stderr/stdout
                log::warn!("green script {} failed!", entry.to_string_lossy());
                log::warn!("reason: {}", String::from_utf8_lossy(&output.stderr));
            }
        }
    }
    Ok(())
}

fn health_check() -> Result<()> {
    let config = GreenbootConfig::get_config();
    log::info!("{config:?}");
    handle_motd("healthcheck is in progress").ok();
    let run_status = run_diagnostics();
    match run_status {
        Ok(()) => {
            log::info!("greenboot health-check passed.");
            run_green().unwrap_or_else(|e| {
                log::error!("cannot run green script due to: {}", e.to_string())
            });
            handle_motd("healthcheck passed - status is GREEN")
                .unwrap_or_else(|e| log::error!("cannot set motd due to : {}", e.to_string()));
            handle_boot_success(true)?;
            Ok(())
        }
        Err(e) => {
            log::error!("Greenboot health-check failed!");
            handle_motd("healthcheck failed - status is RED")
                .unwrap_or_else(|e| log::error!("cannot set motd due to : {}", e.to_string()));
            run_red()
                .unwrap_or_else(|e| log::error!("cannot run red script due to: {}", e.to_string()));
            handle_boot_success(false)?;
            set_boot_counter(config.max_reboot)
                .unwrap_or_else(|e| log::error!("cannot set boot_counter as: {}", e.to_string()));
            handle_reboot(false)
                .unwrap_or_else(|e| log::error!("cannot reboot as: {}", e.to_string()));
            bail!(e);
        }
    }
}

fn trigger_rollback() -> Result<()> {
    match handle_rollback() {
        Ok(()) => {
            log::info!("Rollback successful");
            unset_boot_counter()?;
            handle_reboot(true)?;
            Ok(())
        }
        Err(e) => {
            bail!("Rollback not initiated as {}", e);
        }
    }
}

fn poc_rollback_policy(duration: u32) -> Result<()> {
    let s = Command::new("rpm-ostree")
        .arg("status")
        .arg("--json")
        .output()
        .unwrap();
    let j: serde_json::Value = match str::from_utf8(&s.stdout[..]) {
        Ok(v) => serde_json::from_str(v).unwrap(),
        Err(_) => bail!("cannot_convert to json"),
    };
    let t_current = &j["deployments"][0]["timestamp"];
    let t_current_millis = Duration::from_secs(t_current.as_u64().unwrap());
    let t_previous = &j["deployments"][1]["timestamp"];
    let t_previous_millis = Duration::from_secs(t_previous.as_u64().unwrap());
    if t_current_millis < t_previous_millis {
        bail!("already in the previous deployment");
    }
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap();
    if t_current_millis + Duration::from_secs((duration * 3600).into()) < now {
        bail!("grace prediod has already passed to trigger rollback");
    }
    log::info!("within grace period");
    Ok(())
}

fn poc_service_monitor(mut services: Vec<&str>) -> Result<()> {
    
    //1. check if service exits
    //2. check if services are enabled
    //3. check running
    //4. Reporting

    //this will prioritize after health check and retrun two type of failure 
    //if step 1 or 2 fail then result of 3 is ignored - Critical error, need manual intervention
    //if step 1 and 2 passes but 3 fails - Auto revover error, and regular restart procedure is followed

    //Assumptions
    // let mut services = vec!["sshd","podman"];

    let mut service_not_ok:Vec<&str> = Vec::with_capacity(services.len());
    let mut unforced_error :bool = false; 

    for service in &services {
        match systemctl::exists(service) {
            Ok(service_exists) => {
                if !service_exists {
                    log::warn!("service: {service} does not exits");
                    service_not_ok.push(service);
                } 
            },
            Err(err) => log::error!("Error fetching {service} details: {err}"),
        }
    }
    services.retain(|&v| !service_not_ok.contains(&v));

    for service in &services {
        match systemctl::Unit::from_systemctl(service) {
            Ok(service_details) => {
                match service_details.status(){
                    Ok(state)  => {
                            match state.as_str() {
                                "Enabled" => {
                                    //check for running status...
                                    //will modify unforced_error here
                                    unforced_error=true;
                                },
                                _ => {
                                    service_not_ok.push(service);
                                    log::warn!("service: {} is not enabled", service_details.name);
                                },
                            }; 
                        }
                    Err(err) => log::error!("Error fetching {service} status: {err}"),
                }
            },
            Err(err) => log::error!("Error fetching {service} status: {err}"),
        }
    }

    if !service_not_ok.is_empty() {
        bail!("{}",{
            service_not_ok.dedup();
            ().len()
        });
    }

    if unforced_error {
        bail!("{}", -1);
    }
    Ok(())
}
fn main() -> Result<()> {
    let cli = Cli::parse();
    pretty_env_logger::formatted_builder()
        .filter_level(cli.log_level.to_log())
        .init();

    //get_config should be here.....
    let services = vec!["sshd","podman"];
    
    match cli.command {
        Commands::HealthCheck => health_check(),
        Commands::Rollback => trigger_rollback(),
        Commands::PocRollback => poc_rollback_policy(1),
        Commands::PocServiceMinitor => poc_service_monitor(services),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use anyhow::Context;

    use super::*;

    //validate when required folder is not found
    #[test]
    fn missing_required_folder() {
        assert_eq!(
            run_diagnostics().unwrap_err().to_string(),
            String::from("required.d not found")
        );
    }

    #[test]
    fn test_passed_diagnostics() {
        setup_folder_structure(true)
            .context("Test setup failed")
            .unwrap();
        let state = run_diagnostics();
        assert!(state.is_ok());
        tear_down().context("Test teardown failed").unwrap();
    }

    #[test]
    fn test_failed_diagnostics() {
        setup_folder_structure(false)
            .context("Test setup failed")
            .unwrap();
        let failed_msg = run_diagnostics().unwrap_err().to_string();
        assert_eq!(failed_msg, String::from("health-check failed!"));
        tear_down().context("Test teardown failed").unwrap();
    }

    #[test]
    fn test_boot_counter_set() {
        unset_boot_counter().ok();
        set_boot_counter(10).ok();
        assert_eq!(get_boot_counter(), Some(10));
        unset_boot_counter().ok();
    }

    #[test]
    fn test_boot_counter_re_set() {
        unset_boot_counter().ok();
        set_boot_counter(10).ok();
        set_boot_counter(20).ok();
        assert_eq!(get_boot_counter(), Some(10));
        unset_boot_counter().ok();
    }

    fn setup_folder_structure(passing: bool) -> Result<()> {
        let required_path = format!("{}/check/required.d", GREENBOOT_INSTALL_PATHS[1]);
        let wanted_path = format!("{}/check/wanted.d", GREENBOOT_INSTALL_PATHS[1]);
        let passing_test_scripts = "testing_assets/passing_script.sh";
        let failing_test_scripts = "testing_assets/failing_script.sh";

        fs::create_dir_all(&required_path).expect("cannot create folder");
        fs::create_dir_all(&wanted_path).expect("cannot create folder");
        let _a = fs::copy(
            passing_test_scripts,
            format!("{}/passing_script.sh", &required_path),
        )
        .context("unable to copy test assets");

        let _a = fs::copy(
            passing_test_scripts,
            format!("{}/passing_script.sh", &wanted_path),
        )
        .context("unable to copy test assets");

        let _a = fs::copy(
            failing_test_scripts,
            format!("{}/failing_script.sh", &wanted_path),
        )
        .context("unable to copy test assets");

        if !passing {
            let _a = fs::copy(
                failing_test_scripts,
                format!("{}/failing_script.sh", &required_path),
            )
            .context("unable to copy test assets");
            return Ok(());
        }
        Ok(())
    }

    fn tear_down() -> Result<()> {
        fs::remove_dir_all(GREENBOOT_INSTALL_PATHS[1]).expect("Unable to delete folder");
        Ok(())
    }
}
