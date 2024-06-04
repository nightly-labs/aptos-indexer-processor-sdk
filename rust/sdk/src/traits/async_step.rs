use crate::{connectors::AsyncStepConnector, traits::instrumentation::NamedStep};
use async_trait::async_trait;
use kanal::{AsyncReceiver, AsyncSender};
use std::time::Duration;
use tokio::task::JoinHandle;

#[async_trait]
pub trait AsyncStep
where
    Self: NamedStep + Sized + Send + 'static,
{
    type Input: Send + 'static;
    type Output: Send + 'static;

    /// Processes a batch of input items and returns a batch of output items.
    async fn process(&mut self, items: Vec<Self::Input>) -> Vec<Self::Output>;

    fn connect<NextStep>(self, next_step: NextStep) -> AsyncStepConnector<Self, NextStep>
    where
        NextStep: AsyncStep<Input = Self::Output>,
    {
        AsyncStepConnector {
            first_step: self,
            second_step: next_step,
        }
    }
}

#[async_trait]
pub trait AsyncStepWithInput: AsyncStep
where
    Self: AsyncStep + Sized + Send + 'static,
{
    /// Returns the input channel for receiving input items.
    fn input_receiver(&mut self) -> &AsyncReceiver<Vec<Self::Input>>;
}

#[async_trait]
pub trait AsyncStepWithOutput: AsyncStep
where
    Self: AsyncStep + Sized + Send + 'static,
{
    /// Returns the output channel for sending output items.
    fn output_sender(&mut self) -> &AsyncSender<Vec<Self::Output>>;
}

#[async_trait]
#[allow(dead_code)]
pub trait PollableAsyncStep
where
    Self: Sized + Send + 'static + AsyncStep,
{
    /// Returns the duration between poll attempts.
    fn poll_interval(&self) -> Duration;

    /// Polls the internal state and returns a batch of output items if available.
    async fn poll(&mut self) -> Option<Vec<<Self as AsyncStep>::Output>>;
}

// TODO: Implement this for everything we can automatically?
pub trait SpawnsPollable: PollableAsyncStep + AsyncStepWithInput + AsyncStepWithOutput {
    fn spawn(mut self) -> JoinHandle<()> {
        tokio::spawn(async move {
            let input_receiver = self.input_receiver().clone();
            let output_sender = self.output_sender().clone();
            let poll_duration = self.poll_interval();

            let mut last_poll = tokio::time::Instant::now();

            loop {
                // It's possible that the channel always has items, so we need to ensure we call `poll` manually if we need to
                if last_poll.elapsed() >= poll_duration {
                    let result = self.poll().await;
                    if let Some(output) = result {
                        output_sender
                            .send(output)
                            .await
                            .expect("Failed to send output");
                    };
                    last_poll = tokio::time::Instant::now();
                }

                tokio::select! {
                    _ = tokio::time::sleep(poll_duration) => {
                        let result = self.poll().await;
                        if let Some(output) = result {
                            output_sender.send(output).await.expect("Failed to send output");
                        };
                        last_poll = tokio::time::Instant::now();
                    }
                    input = input_receiver.recv() => {
                        let input = input.expect("Failed to receive input");
                        let output = self.process(input).await;
                        if !output.is_empty() {
                            output_sender.send(output).await.expect("Failed to send output");
                        }
                    }
                }
            }
        })
    }
}

/// Spawns without polling
pub trait SpawnsAsync: AsyncStep + AsyncStepWithInput + AsyncStepWithOutput {
    fn spawn(mut self) -> JoinHandle<()> {
        tokio::spawn(async move {
            let input_receiver = self.input_receiver().clone();
            let output_sender = self.output_sender().clone();

            loop {
                tokio::select! {
                    input = input_receiver.recv() => {
                        let input = input.expect("Failed to receive input");
                        let output = self.process(input).await;
                        if !output.is_empty() {
                            output_sender.send(output).await.expect("Failed to send output");
                        }
                    }
                }
            }
        })
    }
}

/// Spawns pollable with only output sender
pub trait SpawnsPollableWithOutput: PollableAsyncStep + AsyncStepWithOutput {
    fn spawn(mut self) -> JoinHandle<()> {
        tokio::spawn(async move {
            let output_sender = self.output_sender().clone();
            let poll_duration = self.poll_interval();
            loop {
                tokio::select! {
                    _ = tokio::time::sleep(poll_duration) => {
                        let result = self.poll().await;
                        if let Some(output) = result {
                            output_sender.send(output).await.expect("Failed to send output");
                        };
                    }
                }
            }
        })
    }
}
