use std::{env, path::PathBuf};

use anyhow::bail;
use clap::Parser;
use futures::future;
use libc_constant_deprecator_lib::ConstContainer;
use tokio::{
    sync::oneshot::{self, Receiver, Sender},
    task,
};

#[derive(Debug, Parser)]
pub(crate) struct Args {
    path: Option<String>,
}

#[expect(clippy::unused_async, reason = "WIP.")]
pub(crate) async fn render(constants: &mut ConstContainer, rx: Receiver<()>) -> anyhow::Result<()> {
    Ok(())
}

pub(crate) async fn terminator(tx: Sender<()>) -> anyhow::Result<()> {
    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let files = if let Some(path) = Args::parse().path {
        libc_constant_deprecator_lib::scan_files(PathBuf::from(path)).await?
    } else {
        libc_constant_deprecator_lib::scan_files(env::current_dir()?).await?
    };

    let mut parsed_constants = libc_constant_deprecator_lib::parse_constants(&files);

    loop {
        let (terminate_tx, terminate_rx) = oneshot::channel();

        match future::try_join(
            task::spawn(render(&mut parsed_constants, terminate_rx)),
            task::spawn(terminator(terminate_tx)),
        )
        .await
        {
            Ok((Ok(_), Ok(_))) => (),
            Ok((Err(err), _) | (_, Err(err))) => bail!(err),
            Err(task_err) => bail!(task_err),
        }
    }

    Ok(())
}
