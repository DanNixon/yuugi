use clap::Parser;
use kagiyama::{AlwaysReady, Watcher};
use prometheus_client::{
    encoding::text::Encode,
    metrics::{counter::Counter, family::Family, info::Info},
    registry::Unit,
};
use std::{fs, sync::atomic::Ordering};
use sysinfo::{CpuExt, Pid, ProcessExt, System, SystemExt};
use tokio::time::{self, Duration};

#[derive(Parser)]
#[clap(author, version, about, long_about = None)]
struct Cli {
    /// Address on which to serve observability endpoints.
    #[clap(
        short,
        long,
        value_parser,
        env = "METRICS_ADDRESS",
        default_value = "127.0.0.1:9090"
    )]
    metrics_address: String,

    /// Interval in milliseconds at which to collect process information.
    #[clap(
        short,
        long,
        value_parser,
        env = "COLLECTION_INTERVAL",
        default_value = "100"
    )]
    collection_interval: u64,

    /// Average power consumption of the CPU die in Watts.
    /// Can be assumed to be the CPUs TDP if the system is well utilised (i.e. most cores active at
    /// close to the upper frequency).
    #[clap(
        short,
        long,
        value_parser,
        env = "AVERAGE_DIE_POWER",
        default_value = "35"
    )]
    average_die_power: f64,
}

fn get_process_jiffies(pid: &Pid) -> u64 {
    match fs::read_to_string(format!("/proc/{}/stat", pid)) {
        Ok(contents) => {
            let contents: Vec<&str> = contents.split(' ').collect();
            let utime: u64 = contents[13].parse().unwrap();
            let stime: u64 = contents[14].parse().unwrap();
            log::trace!("PID {}: user={} kernel={}", pid, utime, stime);
            utime + stime
        }
        Err(e) => {
            log::warn!("Failed to get process time PID={}, err: {}", pid, e);
            0
        }
    }
}

#[derive(Clone, Hash, PartialEq, Eq, Encode)]
struct Labels {
    process_name: String,
    cmdline: String,
    pid: String,
}

#[tokio::main]
async fn main() {
    env_logger::init();

    let args = Cli::parse();

    let mut watcher = Watcher::<AlwaysReady>::default();
    watcher
        .start_server(args.metrics_address.parse().unwrap())
        .await
        .unwrap();

    let mut sys = System::new_all();
    sys.refresh_all();

    // TODO: Discover this value from CPU model (for TDP) or via a suitable API for CPUs that
    // support measuring actual values
    let average_die_power = args.average_die_power;

    let num_physical_cores = num_cpus::get_physical();
    let average_core_power = average_die_power / (num_physical_cores as f64);

    let cpu_time = Family::<Labels, Counter>::default();
    let energy = Family::<Labels, Counter>::default();

    let clk_tck = sysconf::raw::sysconf(sysconf::raw::SysconfVariable::ScClkTck).unwrap();
    let jiffy_in_seconds = 1.0 / (clk_tck as f64);
    log::info!("1 jiffy is {} seconds", jiffy_in_seconds);

    {
        let mut registry = watcher.metrics_registry();
        let registry =
            registry.sub_registry_with_label(("hostname".into(), sys.host_name().unwrap().into()));

        let system = Info::new(vec![
            (
                "os".to_string(),
                sys.name().unwrap_or_else(|| "unknown".into()),
            ),
            (
                "os_version".to_string(),
                sys.os_version().unwrap_or_else(|| "unknown".into()),
            ),
            (
                "kernel_version".to_string(),
                sys.kernel_version().unwrap_or_else(|| "unknown".into()),
            ),
            ("jiffy_in_seconds".to_string(), jiffy_in_seconds.to_string()),
        ]);
        registry.register("system", "Host OS information", Box::new(system));

        let cpu = Info::new(vec![
            (
                "vendor".to_string(),
                sys.global_cpu_info().vendor_id().to_string(),
            ),
            (
                "model".to_string(),
                sys.global_cpu_info().brand().to_string(),
            ),
            (
                "average_die_power".to_string(),
                average_die_power.to_string(),
            ),
            (
                "average_core_power".to_string(),
                average_core_power.to_string(),
            ),
            (
                "num_physical_cores".to_string(),
                num_physical_cores.to_string(),
            ),
        ]);
        registry.register("cpu", "Host CPU information", Box::new(cpu));

        registry.register_with_unit(
            "cpu_time",
            "Total CPU time spent executing process",
            Unit::Seconds,
            Box::new(cpu_time.clone()),
        );

        registry.register_with_unit(
            "energy",
            "Total energy time spent executing process",
            Unit::Other("watt_hours".to_string()),
            Box::new(energy.clone()),
        );
    }

    let mut collection_interval = time::interval(Duration::from_millis(args.collection_interval));

    loop {
        collection_interval.tick().await;

        log::info!("Refreshing metrics");
        sys.refresh_all();

        for (pid, process) in sys.processes() {
            let labels = Labels {
                process_name: process.name().to_string(),
                cmdline: process.cmd().join(" "),
                pid: pid.to_string(),
            };

            let run_time = (get_process_jiffies(pid) as f64) * jiffy_in_seconds;
            log::trace!("PID {} total CPU time = {}", pid, run_time);

            // TODO: this is dropping sub second precision
            cpu_time
                .get_or_create(&labels)
                .inner()
                .store(run_time as u64, Ordering::Relaxed);

            let e = (run_time * average_core_power) / 3600.0;

            // TODO: this is dropping sub Wh precision
            energy
                .get_or_create(&labels)
                .inner()
                .store(e as u64, Ordering::Relaxed);
        }
    }
}
