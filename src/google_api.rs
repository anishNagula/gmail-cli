use anyhow::{Context, Result};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use dirs::config_dir;
use html2text;
use oauth2::{
    basic::{BasicClient, BasicTokenType},
    AuthUrl, AuthorizationCode, ClientId, ClientSecret, CsrfToken, EmptyExtraTokenFields,
    PkceCodeChallenge, RedirectUrl, Scope, StandardTokenResponse, TokenUrl, TokenResponse,
};
use serde::{Deserialize, Serialize};
use std::fs;
use tokio::io::{stdin, AsyncBufReadExt, BufReader};

const GMAIL_API_TOKEN_PATH: &str = "gmail-cli/token.json";
const GOOGLE_AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const GOOGLE_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ApiToken {
    pub access_token: String,
    pub refresh_token: Option<String>,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct MessageList {
    pub messages: Option<Vec<Message>>,
    pub next_page_token: Option<String>,
    pub result_size_estimate: Option<i32>,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    pub id: String,
    pub thread_id: String,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct MessageDetail {
    pub id: String,
    pub snippet: String,
    pub payload: Option<MessagePayload>,
    pub label_ids: Option<Vec<String>>,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct MessagePayload {
    pub headers: Vec<MessageHeader>,
    pub body: Option<MessageBody>,
    pub parts: Option<Vec<MessagePayload>>,
    pub mime_type: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct MessageBody {
    pub data: Option<String>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct MessageHeader {
    pub name: String,
    pub value: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ModifyRequest {
    remove_label_ids: Vec<String>,
}

impl MessageDetail {
    pub fn get_header(&self, name: &str) -> String {
        self.payload
            .as_ref()
            .and_then(|p| {
                p.headers
                    .iter()
                    .find(|h| h.name.eq_ignore_ascii_case(name))
            })
            .map_or_else(String::new, |h| h.value.clone())
    }

    pub fn is_unread(&self) -> bool {
        if let Some(labels) = &self.label_ids {
            labels.contains(&"UNREAD".to_string())
        } else {
            false
        }
    }
}

pub async fn get_auth_token() -> Result<ApiToken> {
    let token = read_token_from_file().await?;
    match token {
        Some(token) => Ok(token),
        None => {
            let token_response = get_new_token_from_auth_code().await?;
            let api_token = ApiToken {
                access_token: token_response.access_token().secret().clone(),
                refresh_token: token_response.refresh_token().map(|t| t.secret().clone()),
            };
            save_token_to_file(&api_token).await?;
            Ok(api_token)
        }
    }
}

async fn get_new_token_from_auth_code() -> Result<StandardTokenResponse<EmptyExtraTokenFields, BasicTokenType>> {
    // Remember to replace these with your own credentials
    let client_id = ClientId::new("48246542160-fom37e06toart56nlvlt6sta16m7l2pj.apps.googleusercontent.com".to_string());
    let client_secret = ClientSecret::new("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".to_string());
    let auth_url = AuthUrl::new(GOOGLE_AUTH_URL.to_string())?;
    let token_url = TokenUrl::new(GOOGLE_TOKEN_URL.to_string())?;

    let client = BasicClient::new(client_id, Some(client_secret), auth_url, Some(token_url))
        .set_redirect_uri(RedirectUrl::new("http://localhost".to_string())?);

    let (pkce_challenge, pkce_verifier) = PkceCodeChallenge::new_random_sha256();

    let (auth_url, _csrf_token) = client
        .authorize_url(CsrfToken::new_random)
        .add_scope(Scope::new("https://www.googleapis.com/auth/gmail.readonly".to_string()))
        .add_scope(Scope::new("https://www.googleapis.com/auth/gmail.modify".to_string()))
        .set_pkce_challenge(pkce_challenge)
        .url();

    println!("Open this URL in your browser to authorize this app: {}", auth_url);
    println!("Paste the authorization code from the redirected URL below:");

    let mut reader = BufReader::new(stdin()).lines();
    let code_string = reader.next_line().await?.context("Failed to read authorization code")?;
    let code = AuthorizationCode::new(code_string);

    let token_response = client
        .exchange_code(code)
        .set_pkce_verifier(pkce_verifier)
        .request_async(oauth2::reqwest::async_http_client)
        .await?;

    Ok(token_response)
}

pub async fn list_messages(token: &ApiToken) -> Result<MessageList> {
    let client = reqwest::Client::new();
    let url = "https://www.googleapis.com/gmail/v1/users/me/messages?maxResults=50&q=in:inbox category:primary newer_than:30d";
    let res = client
        .get(url)
        .bearer_auth(&token.access_token)
        .send()
        .await?
        .json::<MessageList>()
        .await?;

    Ok(res)
}

pub async fn get_message_headers(token: &ApiToken, message_id: &str) -> Result<MessageDetail> {
    let client = reqwest::Client::new();
    let url = format!(
        "https://www.googleapis.com/gmail/v1/users/me/messages/{}?format=metadata&metadataHeaders=Subject&metadataHeaders=From",
        message_id
    );

    let res = client
        .get(&url)
        .bearer_auth(&token.access_token)
        .send()
        .await?
        .json::<MessageDetail>()
        .await?;
    Ok(res)
}

pub async fn get_full_message(token: &ApiToken, message_id: &str) -> Result<MessageDetail> {
    let client = reqwest::Client::new();
    let url = format!(
        "https://www.googleapis.com/gmail/v1/users/me/messages/{}?format=full",
        message_id
    );

    let res = client
        .get(&url)
        .bearer_auth(&token.access_token)
        .send()
        .await?
        .json::<MessageDetail>()
        .await?;
    Ok(res)
}

pub async fn mark_as_read(token: &ApiToken, message_id: &str) -> Result<()> {
    let client = reqwest::Client::new();
    let url = format!(
        "https://www.googleapis.com/gmail/v1/users/me/messages/{}/modify",
        message_id
    );

    let request_body = ModifyRequest {
        remove_label_ids: vec!["UNREAD".to_string()],
    };

    client
        .post(&url)
        .bearer_auth(&token.access_token)
        .json(&request_body)
        .send()
        .await?
        .error_for_status()?;

    Ok(())
}

fn find_body_parts(payload: &MessagePayload) -> (Option<String>, Option<String>) {
    let mut plain_text = None;
    let mut html_text = None;

    if payload.mime_type == "text/plain" {
        if let Some(data) = payload.body.as_ref().and_then(|b| b.data.as_ref()) {
            if let Ok(decoded_bytes) = URL_SAFE_NO_PAD.decode(data) {
                plain_text = String::from_utf8(decoded_bytes).ok();
            }
        }
    } else if payload.mime_type == "text/html" {
        if let Some(data) = payload.body.as_ref().and_then(|b| b.data.as_ref()) {
            if let Ok(decoded_bytes) = URL_SAFE_NO_PAD.decode(data) {
                html_text = String::from_utf8(decoded_bytes).ok();
            }
        }
    }

    if let Some(parts) = &payload.parts {
        for part in parts {
            let (part_plain, part_html) = find_body_parts(part);
            if plain_text.is_none() {
                plain_text = part_plain;
            }
            if html_text.is_none() {
                html_text = part_html;
            }
        }
    }

    (plain_text, html_text)
}

pub fn decode_email_body(detail: &MessageDetail) -> String {
    if let Some(payload) = &detail.payload {
        let (plain, html) = find_body_parts(payload);

        if let Some(plain_text) = plain {
            return plain_text;
        }

        if let Some(html_text) = html {
            return html2text::from_read(html_text.as_bytes(), 80);
        }
    }

    detail.snippet.clone()
}

async fn read_token_from_file() -> Result<Option<ApiToken>> {
    if let Some(mut path) = config_dir() {
        path.push(GMAIL_API_TOKEN_PATH);
        if path.exists() {
            let content = fs::read_to_string(&path)?;
            let token: ApiToken = serde_json::from_str(&content)?;
            return Ok(Some(token));
        }
    }
    Ok(None)
}

async fn save_token_to_file(token: &ApiToken) -> Result<()> {
    if let Some(mut path) = config_dir() {
        path.push(GMAIL_API_TOKEN_PATH);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let content = serde_json::to_string_pretty(token)?;
        fs::write(&path, content)?;
    }
    Ok(())
}