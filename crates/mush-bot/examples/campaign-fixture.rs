#[path = "../tests/support/campaign_fixture.rs"]
mod fixture;

#[tokio::main]
async fn main() -> mush_bot::Result<()> {
    let path = std::env::args()
        .nth(1)
        .or_else(|| std::env::var("MUSH_CAMPAIGN_START").ok())
        .ok_or("pass a fixture-start JSON path or set MUSH_CAMPAIGN_START")?;
    fixture::serve(path.into()).await
}
