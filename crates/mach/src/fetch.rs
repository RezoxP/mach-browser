//! `mach fetch` subcommand.

use mach_core::{Config, Error};
use mach_net::HttpClient;
use mach_profile::Registry;

use crate::cli::{DumpFormat, FetchArgs};

pub async fn run(args: FetchArgs, mut config: Config) -> Result<(), Error> {
    // Per-subcommand overrides on the shared config.
    config.http_timeout = args.timeout_dur();
    config.user_agent_override = args.user_agent.clone();

    let profile = if let Some(id) = args.profile.as_deref() {
        Registry::get(id).ok_or_else(|| {
            Error::InvalidArguments(format!(
                "unknown profile {id:?}; known: {known:?}",
                known = Registry::all().iter().map(|p| p.id).collect::<Vec<_>>()
            ))
        })?
    } else {
        Registry::default_profile()
    };

    let client = HttpClient::new(profile, config.http_timeout)?;
    let resp = client.get(&args.url).await?;
    if !(200..400).contains(&resp.status) {
        return Err(Error::Network(format!(
            "HTTP {} for {}",
            resp.status, resp.final_url
        )));
    }

    let doc = mach_parser::parse_html(&resp.body)?;

    let out = match args.dump {
        DumpFormat::Html => doc.serialize_html(),
        DumpFormat::Markdown => mach_agent::markdown::render(&doc),
        DumpFormat::Links => mach_agent::links::collect(&doc, &resp.final_url).join("\n"),
        DumpFormat::Text => mach_agent::text::render(&doc),
    };

    println!("{out}");
    Ok(())
}
