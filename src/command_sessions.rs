use std::any::Any;
use async_trait::async_trait;

#[async_trait]
pub trait CommandSession: Any + Send + Sync 
{
    fn as_any_mut(&mut self) -> &mut dyn Any;

    async fn kill_process(&mut self) -> Result<(), String>;
}