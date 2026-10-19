//! Tool-free, bounded direct inference. Provider credentials never enter preferences.
use super::{
    AgentCli, AgentConfig, GeneratedQuery, MAX_OUTPUT, MAX_PROMPT, TIMEOUT, parse_response,
};
use secrecy::{ExposeSecret, SecretString};
use serde_json::{Value, json};

fn endpoint(cli: AgentCli, config: &AgentConfig) -> Result<url::Url, String> {
    let raw = if cli == AgentCli::Ollama && !config.executable.trim().is_empty() {
        config.executable.trim()
    } else {
        cli.executable()
    };
    let mut url = url::Url::parse(raw).map_err(|_| "Enter a valid Ollama HTTP URL")?;
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(
            "The assistant endpoint must not contain credentials, a query or a fragment.".into(),
        );
    }
    let loopback = url.host_str().is_some_and(|host| {
        host == "localhost"
            || host
                .trim_matches(['[', ']'])
                .parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_loopback())
    });
    if url.scheme() != "https" && !(url.scheme() == "http" && loopback) {
        return Err("Use HTTPS for remote models; HTTP is supported only on loopback.".into());
    }
    if cli == AgentCli::Ollama {
        url.set_path(&format!(
            "{}/api/generate",
            url.path().trim_end_matches('/')
        ));
    }
    Ok(url)
}

fn request_body(cli: AgentCli, model: &str, prompt: &str) -> Value {
    match cli {
        AgentCli::OpenAiApi => {
            json!({"model":model,"input":prompt,"store":false,"max_output_tokens":8192})
        }
        AgentCli::AnthropicApi => {
            json!({"model":model,"max_tokens":8192,"messages":[{"role":"user","content":prompt}]})
        }
        AgentCli::Ollama => {
            json!({"model":model,"prompt":prompt,"stream":false,"format":"json","options":{"num_predict":8192}})
        }
        _ => Value::Null,
    }
}

fn response_text(cli: AgentCli, value: &Value) -> Result<String, String> {
    if value.get("error").is_some_and(|error| !error.is_null()) {
        return Err("The model provider returned an error. Check the model, credentials and provider status.".into());
    }
    let text = match cli {
        AgentCli::OpenAiApi => {
            if value["status"]
                .as_str()
                .is_some_and(|status| status != "completed")
            {
                return Err(
                    "The model response was incomplete. Try a shorter request or another model."
                        .into(),
                );
            }
            value["output"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|item| item["type"] == "message")
                .flat_map(|item| item["content"].as_array().into_iter().flatten())
                .filter(|part| part["type"] == "output_text")
                .filter_map(|part| part["text"].as_str())
                .collect::<String>()
        }
        AgentCli::AnthropicApi => {
            if value["stop_reason"] == "max_tokens" {
                return Err("The model reached its output limit. Try a shorter request.".into());
            }
            value["content"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|part| part["type"] == "text")
                .filter_map(|part| part["text"].as_str())
                .collect::<String>()
        }
        AgentCli::Ollama => value["response"].as_str().unwrap_or_default().to_owned(),
        _ => return Err("Select a direct API provider.".into()),
    };
    if text.trim().is_empty() {
        return Err("The model returned no query text (or declined the request).".into());
    }
    Ok(text)
}

pub async fn generate_direct(
    cli: AgentCli,
    config: AgentConfig,
    key: Option<SecretString>,
    prompt: String,
) -> Result<GeneratedQuery, String> {
    if !cli.is_direct() || config.model.trim().is_empty() {
        return Err(
            "Choose a provider and enter an available model in Query assistant settings.".into(),
        );
    }
    if prompt.len() > MAX_PROMPT {
        return Err("The request exceeds the assistant's schema/prompt limit.".into());
    }
    let endpoint = endpoint(cli, &config)?;
    let client = reqwest::Client::builder()
        .timeout(TIMEOUT)
        .connect_timeout(std::time::Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "Could not initialize the model connection")?;
    let mut request = client
        .post(endpoint)
        .json(&request_body(cli, config.model.trim(), &prompt));
    if cli != AgentCli::Ollama {
        let key = key
            .as_ref()
            .filter(|key| !key.expose_secret().trim().is_empty())
            .ok_or("Save an API key in Query assistant settings first.")?;
        let authorization = if cli == AgentCli::OpenAiApi {
            format!("Bearer {}", key.expose_secret())
        } else {
            key.expose_secret().to_owned()
        };
        let mut header = reqwest::header::HeaderValue::from_str(&authorization)
            .map_err(|_| "The API key contains invalid header characters")?;
        header.set_sensitive(true);
        if cli == AgentCli::OpenAiApi {
            request = request.header(reqwest::header::AUTHORIZATION, header);
        } else {
            request = request
                .header("x-api-key", header)
                .header("anthropic-version", "2023-06-01");
        }
    }
    let mut response = request.send().await.map_err(
        |_| "Could not reach the model provider. Check its endpoint and network connection.",
    )?;
    if !response.status().is_success() {
        return Err(format!(
            "Model provider returned HTTP {}. Check the key, model, quota and provider status.",
            response.status().as_u16()
        ));
    }
    if response
        .content_length()
        .is_some_and(|size| size > MAX_OUTPUT as u64)
    {
        return Err("Model response exceeded 1 MiB.".into());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "The model response was interrupted")?
    {
        if bytes.len().saturating_add(chunk.len()) > MAX_OUTPUT {
            return Err("Model response exceeded 1 MiB.".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    let value: Value =
        serde_json::from_slice(&bytes).map_err(|_| "Model provider returned invalid JSON")?;
    parse_response(&response_text(cli, &value)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn ollama_works_without_any_cli_and_returns_a_draft() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            let mut chunk = [0; 4096];
            loop {
                let read = socket.read(&mut chunk).await.unwrap();
                assert!(read > 0);
                bytes.extend_from_slice(&chunk[..read]);
                if let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&bytes[..end]).to_ascii_lowercase();
                    let length: usize = headers
                        .lines()
                        .find_map(|line| line.strip_prefix("content-length: "))
                        .unwrap()
                        .parse()
                        .unwrap();
                    if bytes.len() >= end + 4 + length {
                        assert!(headers.starts_with("post /api/generate "));
                        assert!(!headers.contains("authorization"));
                        let body: Value = serde_json::from_slice(&bytes[end + 4..]).unwrap();
                        assert_eq!(body["model"], "fixture-model");
                        assert_eq!(body["stream"], false);
                        assert!(body.get("tools").is_none());
                        break;
                    }
                }
            }
            let response =
                json!({"response":r#"{"query":"SELECT 42","explanation":"A draft"}"#, "done":true})
                    .to_string();
            socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}",response.len()).as_bytes()).await.unwrap();
        });
        let result = generate_direct(
            AgentCli::Ollama,
            AgentConfig {
                executable: format!("http://{address}"),
                model: "fixture-model".into(),
                provider: String::new(),
            },
            None,
            "Draft a query".into(),
        )
        .await
        .unwrap();
        assert_eq!(result.query, "SELECT 42");
        server.await.unwrap();
    }

    #[test]
    fn provider_shapes_extract_only_text_and_never_tools() {
        let result = r#"{"query":"SELECT 1","explanation":"A constant"}"#;
        let openai = json!({"status":"completed","output":[{"type":"function_call","arguments":"DELETE"},{"type":"message","content":[{"type":"output_text","text":result}]}]});
        assert_eq!(
            parse_response(&response_text(AgentCli::OpenAiApi, &openai).unwrap())
                .unwrap()
                .query,
            "SELECT 1"
        );
        let anthropic = json!({"content":[{"type":"text","text":result}],"stop_reason":"end_turn"});
        assert_eq!(
            response_text(AgentCli::AnthropicApi, &anthropic).unwrap(),
            result
        );
        assert_eq!(
            response_text(AgentCli::Ollama, &json!({"response":result})).unwrap(),
            result
        );
        assert!(response_text(AgentCli::OpenAiApi, &json!({"status":"incomplete"})).is_err());
        for cli in [
            AgentCli::OpenAiApi,
            AgentCli::AnthropicApi,
            AgentCli::Ollama,
        ] {
            assert!(request_body(cli, "model", "prompt").get("tools").is_none());
        }
    }

    #[test]
    fn credentials_cannot_be_redirected_or_embedded_in_endpoint() {
        for raw in [
            "http://remote.test",
            "http://key@localhost:11434",
            "file:///tmp/model",
            "https://host/?token=secret",
        ] {
            assert!(
                endpoint(
                    AgentCli::Ollama,
                    &AgentConfig {
                        executable: raw.into(),
                        ..Default::default()
                    }
                )
                .is_err()
            );
        }
        let config = AgentConfig {
            executable: "http://localhost:11434".into(),
            ..Default::default()
        };
        assert_eq!(
            endpoint(AgentCli::Ollama, &config).unwrap().as_str(),
            "http://localhost:11434/api/generate"
        );
        assert_eq!(
            endpoint(AgentCli::OpenAiApi, &config).unwrap().host_str(),
            Some("api.openai.com")
        );
    }
}
