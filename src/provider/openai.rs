//! A client for any endpoint compatible with the OpenAI Chat Completions API.

use std::time::Duration;

use anyhow::{anyhow, Context as _, Result};
use serde::{Deserialize, Serialize};

use crate::config::Config;
use crate::context::Context;
use crate::prompt;
use crate::provider::Provider;
use crate::suggestion::Suggestion;

pub struct OpenAiProvider {
    client: reqwest::blocking::Client,
    endpoint: String,
    model: String,
    api_key: Option<String>,
    json_mode: bool,
    disable_thinking: bool,
}

impl OpenAiProvider {
    pub fn from_config(config: &Config) -> Result<Self> {
        if config.provider.base_url.trim().is_empty() {
            anyhow::bail!("the configuration has no base_url — run `plz config init`");
        }
        if config.provider.model.trim().is_empty() {
            anyhow::bail!("the configuration has no model — run `plz config init`");
        }

        let api_key = config.api_key();
        if config.key_required() && api_key.is_none() {
            anyhow::bail!(
                "no API key found.\n\
                 Set PLZ_API_KEY or run `plz config init`."
            );
        }

        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(config.behavior.timeout_secs))
            .user_agent(concat!("plz/", env!("CARGO_PKG_VERSION")))
            .build()
            .context("could not build the HTTP client")?;

        Ok(Self {
            client,
            endpoint: format!(
                "{}/chat/completions",
                config.provider.base_url.trim_end_matches('/')
            ),
            model: config.provider.model.clone(),
            api_key,
            json_mode: config.behavior.json_mode,
            disable_thinking: config.behavior.disable_thinking,
        })
    }

    /// Override the model for a single run (the `--model` flag).
    pub fn with_model(mut self, model: Option<String>) -> Self {
        if let Some(model) = model {
            self.model = model;
        }
        self
    }

    fn post(&self, request: &ChatRequest) -> Result<ChatResponse> {
        let mut builder = self.client.post(&self.endpoint).json(request);
        if let Some(key) = &self.api_key {
            builder = builder.bearer_auth(key);
        }

        let response = builder
            .send()
            .map_err(|err| describe_transport_error(err, &self.endpoint))?;

        let status = response.status();
        let body = response
            .text()
            .context("could not read the server's response")?;

        if !status.is_success() {
            return Err(describe_api_error(status, &body));
        }

        serde_json::from_str::<ChatResponse>(&body)
            .with_context(|| format!("unexpected response format from {}", self.endpoint))
    }
}

impl Provider for OpenAiProvider {
    fn suggest(&self, ctx: &Context, task: &str, count: usize) -> Result<Vec<Suggestion>> {
        let request = ChatRequest {
            model: &self.model,
            messages: vec![
                Message {
                    role: "system",
                    content: prompt::system_prompt(ctx, count),
                },
                Message {
                    role: "user",
                    content: prompt::user_prompt(task),
                },
            ],
            // Only ask for the format where the endpoint supports it: Ollama and
            // some compatible servers answer 400 on an unknown field.
            response_format: self.json_mode.then_some(ResponseFormat {
                kind: "json_object",
            }),
            // Likewise: the field is DeepSeek's, not the OpenAI API's. One
            // command is not worth a chain of reasoning, and a thinking model
            // sometimes returns it instead of the JSON that was asked for.
            thinking: self
                .disable_thinking
                .then_some(Thinking { kind: "disabled" }),
        };

        let response = self.post(&request)?;
        let content = response
            .choices
            .into_iter()
            .next()
            .map(|c| c.message.content)
            .ok_or_else(|| anyhow!("the model returned an empty response"))?;

        let mut suggestions = prompt::parse_suggestions(&content)?;
        suggestions.truncate(count);
        Ok(suggestions)
    }
}

/// Turn a transport error into a hint about what to fix.
fn describe_transport_error(err: reqwest::Error, endpoint: &str) -> anyhow::Error {
    if err.is_timeout() {
        return anyhow!(
            "the request to {endpoint} timed out.\n\
             Raise `timeout_secs` in the configuration or check your connection."
        );
    }
    if err.is_connect() {
        return anyhow!(
            "could not connect to {endpoint}: {err}.\n\
             Check base_url and your network (for Ollama, whether the server is running)."
        );
    }
    anyhow!("request to {endpoint} failed: {err}")
}

/// Turn an HTTP error from the API into a human-readable message.
fn describe_api_error(status: reqwest::StatusCode, body: &str) -> anyhow::Error {
    let detail = extract_api_message(body).unwrap_or_else(|| snippet(body));

    let hint = match status.as_u16() {
        401 | 403 => "The key is invalid or expired. Check PLZ_API_KEY or run `plz config init`.",
        404 => "Check base_url and the model name — the provider may not have that model.",
        429 => "Rate limit exceeded. Wait, or check your account balance.",
        400 => {
            "Request rejected. If the endpoint does not support json_object, \
                set `json_mode = false` in the configuration."
        }
        500..=599 => "The provider had a server-side error. Try again later.",
        _ => "",
    };

    if hint.is_empty() {
        anyhow!("the API returned {status}: {detail}")
    } else {
        anyhow!("the API returned {status}: {detail}\n{hint}")
    }
}

/// Pull `error.message` out of an error body when it is there.
fn extract_api_message(body: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(body).ok()?;
    let error = value.get("error")?;
    error
        .get("message")
        .and_then(|m| m.as_str())
        .map(str::to_string)
        .or_else(|| error.as_str().map(str::to_string))
}

fn snippet(body: &str) -> String {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return "(empty response)".into();
    }
    let cut: String = trimmed.chars().take(300).collect();
    if cut.chars().count() < trimmed.chars().count() {
        format!("{cut}…")
    } else {
        cut
    }
}

#[derive(Debug, Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: Vec<Message>,
    #[serde(skip_serializing_if = "Option::is_none")]
    response_format: Option<ResponseFormat>,
    #[serde(skip_serializing_if = "Option::is_none")]
    thinking: Option<Thinking>,
}

#[derive(Debug, Serialize)]
struct Message {
    role: &'static str,
    content: String,
}

#[derive(Debug, Serialize)]
struct ResponseFormat {
    #[serde(rename = "type")]
    kind: &'static str,
}

#[derive(Debug, Serialize)]
struct Thinking {
    #[serde(rename = "type")]
    kind: &'static str,
}

#[derive(Debug, Deserialize)]
struct ChatResponse {
    #[serde(default)]
    choices: Vec<Choice>,
}

#[derive(Debug, Deserialize)]
struct Choice {
    message: ResponseMessage,
}

#[derive(Debug, Deserialize)]
struct ResponseMessage {
    #[serde(default)]
    content: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::env_guard;

    #[test]
    fn endpoint_is_built_without_double_slash() {
        let mut config = Config::default();
        config.provider.base_url = "https://api.example.com/v1/".into();
        config.provider.api_key = Some("k".into());
        let provider = OpenAiProvider::from_config(&config).unwrap();
        assert_eq!(
            provider.endpoint,
            "https://api.example.com/v1/chat/completions"
        );
    }

    #[test]
    fn missing_key_is_reported_before_any_request() {
        // The guard matters here more than anywhere: this test asserts the
        // *absence* of keys that config's own tests set at the same moment.
        let _guard = env_guard();
        let mut config = Config::default();
        config.provider.api_key = None;
        // Make sure no key leaks in from the developer's environment.
        std::env::remove_var("PLZ_API_KEY");
        std::env::remove_var("DEEPSEEK_API_KEY");
        let err = OpenAiProvider::from_config(&config)
            .err()
            .unwrap()
            .to_string();
        assert!(err.contains("API key"));
    }

    #[test]
    fn ollama_works_without_a_key() {
        let mut config = Config::default();
        config.provider.preset = crate::provider::presets::OLLAMA.name.into();
        config.provider.base_url = crate::provider::presets::OLLAMA.base_url.into();
        config.provider.model = crate::provider::presets::OLLAMA.model.into();
        config.provider.api_key = None;
        assert!(OpenAiProvider::from_config(&config).is_ok());
    }

    #[test]
    fn empty_base_url_is_rejected() {
        let mut config = Config::default();
        config.provider.base_url = String::new();
        let err = OpenAiProvider::from_config(&config)
            .err()
            .unwrap()
            .to_string();
        assert!(err.contains("base_url"));
    }

    #[test]
    fn api_error_message_is_extracted_from_the_body() {
        let body = r#"{"error":{"message":"Invalid API key","type":"authentication_error"}}"#;
        assert_eq!(
            extract_api_message(body).as_deref(),
            Some("Invalid API key")
        );
    }

    #[test]
    fn unauthorised_error_explains_what_to_fix() {
        let err = describe_api_error(
            reqwest::StatusCode::UNAUTHORIZED,
            r#"{"error":{"message":"bad key"}}"#,
        )
        .to_string();
        assert!(err.contains("bad key"));
        assert!(err.contains("PLZ_API_KEY"));
    }

    #[test]
    fn bad_request_hints_at_json_mode() {
        let err = describe_api_error(reqwest::StatusCode::BAD_REQUEST, "{}").to_string();
        assert!(err.contains("json_mode"));
    }

    #[test]
    fn non_json_error_body_is_shown_as_is() {
        let err = describe_api_error(
            reqwest::StatusCode::INTERNAL_SERVER_ERROR,
            "<html>Bad Gateway</html>",
        )
        .to_string();
        assert!(err.contains("Bad Gateway"));
    }

    /// A request with nothing in it but the model name, for the fields below.
    fn bare_request() -> ChatRequest<'static> {
        ChatRequest {
            model: "m",
            messages: Vec::new(),
            response_format: None,
            thinking: None,
        }
    }

    #[test]
    fn disabled_thinking_is_sent_as_a_typed_field() {
        let request = ChatRequest {
            thinking: Some(Thinking { kind: "disabled" }),
            ..bare_request()
        };
        let body = serde_json::to_string(&request).unwrap();
        assert!(body.contains(r#""thinking":{"type":"disabled"}"#), "{body}");
    }

    #[test]
    fn thinking_is_absent_from_the_body_when_not_asked_for() {
        // The field is not part of the OpenAI API: sending it to an endpoint
        // that does not know it is a 400.
        let body = serde_json::to_string(&bare_request()).unwrap();
        assert!(!body.contains("thinking"), "{body}");
    }

    #[test]
    fn response_without_choices_is_an_error_not_a_panic() {
        let parsed: ChatResponse = serde_json::from_str(r#"{"choices":[]}"#).unwrap();
        assert!(parsed.choices.is_empty());
    }
}
