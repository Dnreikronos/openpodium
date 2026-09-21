use std::collections::BTreeMap;
use std::error::Error;
use std::io::{self, BufRead, Write};
use std::time::Duration;

use openpodium::plugins::{AdapterPlan, CommandOutcome, HostMessage, PluginAction, PluginMessage};

fn main() -> Result<(), Box<dyn Error>> {
    let stdin = io::stdin();
    let mut lines = stdin.lock().lines();
    let stdout = io::stdout();
    let mut output = stdout.lock();

    let Some(line) = lines.next() else {
        return Ok(());
    };
    let hello: HostMessage = serde_json::from_str(&line?)?;
    let sdk_version = match hello {
        HostMessage::HostHello { sdk_version, .. } => sdk_version,
        _ => return Err("the first message was not host_hello".into()),
    };
    send(&mut output, &PluginMessage::PluginReady { sdk_version })?;

    for line in lines {
        let message: HostMessage = serde_json::from_str(&line?)?;
        match message {
            HostMessage::InvokeCommand {
                id,
                command,
                context,
            } if command == "say-hello" => {
                if context.get("crash").and_then(|value| value.as_bool()) == Some(true) {
                    std::process::exit(42);
                }
                if context.get("hang").and_then(|value| value.as_bool()) == Some(true) {
                    std::thread::sleep(Duration::from_secs(30));
                }
                if context.get("malformed").and_then(|value| value.as_bool()) == Some(true) {
                    output.write_all(b"this is not JSON\n")?;
                    output.flush()?;
                    continue;
                }
                let name = context
                    .get("name")
                    .and_then(|value| value.as_str())
                    .unwrap_or("OpenPodium");
                let actions =
                    if context.get("escalate").and_then(|value| value.as_bool()) == Some(true) {
                        vec![PluginAction::SetSetting {
                            key: "undeclared".to_owned(),
                            value: serde_json::json!(true),
                        }]
                    } else {
                        Vec::new()
                    };
                send(
                    &mut output,
                    &PluginMessage::CommandResult {
                        id,
                        outcome: CommandOutcome {
                            value: serde_json::json!({ "message": format!("Hello, {name}!") }),
                            actions,
                        },
                    },
                )?;
            }
            HostMessage::PrepareAdapter {
                id,
                provider,
                request,
            } if provider == "echo" => {
                let mut environment = BTreeMap::new();
                environment.insert(
                    "OPENPODIUM_SAMPLE_MODEL".to_owned(),
                    request.model.unwrap_or_else(|| "echo-1".to_owned()),
                );
                send(
                    &mut output,
                    &PluginMessage::AdapterPlan {
                        id,
                        plan: AdapterPlan {
                            program: "sample-agent".to_owned(),
                            arguments: request.prompt.into_iter().collect(),
                            environment,
                        },
                    },
                )?;
            }
            HostMessage::Shutdown => break,
            HostMessage::HostHello { .. } => {
                return Err("received a second host_hello".into());
            }
            message => {
                let Some(id) = request_id(&message) else {
                    return Err("message did not have a request id".into());
                };
                send(
                    &mut output,
                    &PluginMessage::Error {
                        id,
                        message: "unsupported sample operation".to_owned(),
                    },
                )?;
            }
        }
    }
    Ok(())
}

fn request_id(message: &HostMessage) -> Option<u64> {
    match message {
        HostMessage::InvokeCommand { id, .. }
        | HostMessage::PrepareAdapter { id, .. }
        | HostMessage::DeliverEvent { id, .. }
        | HostMessage::ProvideSettings { id, .. } => Some(*id),
        HostMessage::HostHello { .. } | HostMessage::Shutdown => None,
    }
}

fn send(output: &mut impl Write, message: &PluginMessage) -> Result<(), Box<dyn Error>> {
    serde_json::to_writer(&mut *output, message)?;
    output.write_all(b"\n")?;
    output.flush()?;
    Ok(())
}
