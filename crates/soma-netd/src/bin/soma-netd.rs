//! `soma-netd`: prepare sterile network bundles and serve claim, activate, release, and
//! reconcile over one Unix `SOCK_SEQPACKET` socket.
//!
//! The socket lives in a directory this broker owns; `--socket-group` names the group that may
//! reach it, `--lifecycle-uid` names every peer that may claim, activate, and release, and
//! `--reconcile-uid` names every peer that may reconcile.
//! Only Linux hosts with `CAP_NET_ADMIN` can run the broker; elsewhere it exits with a typed
//! message and no side effect.

use std::process::ExitCode;

#[cfg(target_os = "linux")]
fn run() -> Result<(), String> {
    use std::{
        net::{IpAddr, Ipv4Addr},
        path::PathBuf,
    };

    use soma_netd::{
        Broker, CleanupGeneration, ControlAuthority, InterfaceName, NetworkProfile, SubnetPlan,
        broker_owner, serve,
    };

    let mut socket = PathBuf::from("/run/soma-netd/broker.sock");
    let mut state = PathBuf::from("/run/soma-netd");
    let mut uplink = None;
    let mut prepared = 4_usize;
    let mut resolvers = Vec::new();
    let mut host_addresses = Vec::new();
    let mut generation = 1_u32;
    let mut socket_group = None;
    let mut lifecycle = Vec::new();
    let mut reconcile = Vec::new();
    let mut args = std::env::args().skip(1);
    while let Some(flag) = args.next() {
        let mut value = || args.next().ok_or(format!("{flag} needs a value"));
        match flag.as_str() {
            "--socket" => socket = PathBuf::from(value()?),
            "--state" => state = PathBuf::from(value()?),
            "--uplink" => uplink = Some(value()?),
            "--prepare" => prepared = value()?.parse().map_err(|_| "--prepare must be a count")?,
            "--generation" => {
                generation = value()?
                    .parse()
                    .map_err(|_| "--generation must be a number")?;
            }
            "--socket-group" => {
                socket_group = Some(
                    value()?
                        .parse()
                        .map_err(|_| "--socket-group must be a gid")?,
                );
            }
            "--lifecycle-uid" => {
                lifecycle.push(
                    value()?
                        .parse()
                        .map_err(|_| "--lifecycle-uid must be a uid")?,
                );
            }
            "--reconcile-uid" => {
                reconcile.push(
                    value()?
                        .parse()
                        .map_err(|_| "--reconcile-uid must be a uid")?,
                );
            }
            "--resolver" => resolvers.push(
                value()?
                    .parse::<Ipv4Addr>()
                    .map_err(|_| "--resolver must be an IPv4 address")?,
            ),
            "--host-address" => host_addresses.push(
                value()?
                    .parse::<IpAddr>()
                    .map_err(|_| "--host-address must be an IP address")?,
            ),
            _ => return Err(format!("unknown argument {flag}")),
        }
    }
    let uplink = uplink.ok_or("--uplink is required")?;
    let socket_group = socket_group.ok_or("--socket-group is required")?;
    let authority = ControlAuthority::new(broker_owner(), socket_group, &lifecycle, &reconcile)
        .map_err(|error| error.to_string())?;
    let profile = NetworkProfile::new(
        InterfaceName::new(&uplink).map_err(|error| error.to_string())?,
        SubnetPlan::new(Ipv4Addr::new(10, 200, 0, 0), 16).map_err(|error| error.to_string())?,
        SubnetPlan::new(Ipv4Addr::new(10, 201, 0, 0), 16).map_err(|error| error.to_string())?,
        resolvers,
        &host_addresses,
        &[],
    )
    .map_err(|error| error.to_string())?;
    let generation = CleanupGeneration::new(generation).map_err(|error| error.to_string())?;
    let broker =
        Broker::open(profile, &state, generation, 16_384).map_err(|error| error.to_string())?;
    serve(broker, &socket, prepared, authority).map_err(|error| error.to_string())
}

#[cfg(not(target_os = "linux"))]
fn run() -> Result<(), String> {
    Err("soma-netd requires a Linux host with CAP_NET_ADMIN".to_owned())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("soma-netd: {message}");
            ExitCode::FAILURE
        }
    }
}
