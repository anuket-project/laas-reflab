use crate::dashboard::StatusSentiment;

#[allow(async_fn_in_trait)]
pub trait EasyLog {
    async fn log<H, D>(&self, header: H, detail: D, status: StatusSentiment)
    where
        H: Into<String>,
        D: Into<String>;
}
