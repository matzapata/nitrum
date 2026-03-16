use clap::Args;

#[derive(Args)]
pub struct DescribeArgs {
    /// Resource to describe (e.g. enclave name)
    #[arg(short, long)]
    pub resource: Option<String>,
}

pub fn run(_args: DescribeArgs) {
    // TODO
}
