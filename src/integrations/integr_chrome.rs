use std::any::Any;
use std::sync::Arc;
use std::collections::HashMap;
use std::time::Duration;
use serde_json::Value;
use tokio::sync::{Mutex as AMutex, RwLock as ARwLock};
use async_trait::async_trait;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::ContextEnum;
use crate::integrations::sessions::{IntegrationSession, get_session_hashmap_key};
use crate::global_context::GlobalContext;
use crate::call_validation::{ChatContent, ChatMessage};
use crate::scratchpads::multimodality::MultimodalElement;
use crate::tools::tools_description::Tool;

use reqwest::Client;
use std::path::PathBuf;
use headless_chrome::{Browser, LaunchOptions, Tab};
use headless_chrome::protocol::cdp::Page;
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct IntegrationChrome {
    pub chrome_path: Option<String>,
    pub window_size: Option<Vec<u32>>,
    pub idle_browser_timeout: Option<u32>,
}
pub struct ToolChrome {
    integration_chrome: IntegrationChrome,
}

pub struct ChromeSession {
    #[allow(dead_code)]    // it's not actually useless code, it keeps strong reference on browser so it doesn't die
    browser: Browser,
    tab: Arc<Tab>,
}

impl IntegrationSession for ChromeSession
{
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
    fn is_expired(&self) -> bool { false }
}

impl ToolChrome {
    pub fn new_from_yaml(v: &serde_yaml::Value) -> Result<Self, String> {
        let integration_chrome = serde_yaml::from_value::<IntegrationChrome>(v.clone()).map_err(|e| {
            let location = e.location().map(|loc| format!(" at line {}, column {}", loc.line(), loc.column())).unwrap_or_default();
            format!("{}{}", e.to_string(), location)
        })?;
        Ok(Self { integration_chrome })
    }
}

#[async_trait]
impl Tool for ToolChrome {
    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let command_args = parse_command_args(args)?;

        let (gcx, chat_id) = {
            let ccx_lock = ccx.lock().await;
            (ccx_lock.global_context.clone(), ccx_lock.chat_id.clone())
        };

        let session_hashmap_key = get_session_hashmap_key("chrome", &chat_id);
        start_chrome_session(&self.integration_chrome, &session_hashmap_key, gcx.clone()).await?;
        let messages = interact_with_chrome(&command_args, &session_hashmap_key, &tool_call_id, gcx.clone()).await?;

        Ok((false, messages))
    }

    fn command_to_match_against_confirm_deny(
        &self,
        args: &HashMap<String, Value>,
    ) -> Result<String, String> {
        let command_args = parse_command_args(args)?;
        Ok(command_args.join(" "))
    }
}

fn parse_command_args(args: &HashMap<String, Value>) -> Result<Vec<String>, String> {
    let command = match args.get("command") {
        Some(Value::String(s)) => s,
        Some(v) => return Err(format!("argument `command` is not a string: {:?}", v)),
        None => return Err("Missing argument `command`".to_string())
    };

    let parsed_args = shell_words::split(&command).map_err(|e| e.to_string())?;
    if parsed_args.is_empty() {
        return Err("Parsed command is empty".to_string());
    }

    Ok(parsed_args)
}

async fn start_chrome_session(
    args: &IntegrationChrome,
    session_hashmap_key: &String,
    gcx: Arc<ARwLock<GlobalContext>>) -> Result<bool, String>
{
    if !is_chrome_session_active(&session_hashmap_key, gcx.clone()).await {
        let mut is_connection = false;
        if let Some(chrome_path) = args.chrome_path.clone() {
            is_connection = chrome_path.starts_with("ws://");
        }
        let mut window_size: Option<(u32, u32)> = None;
        if let Some(size) = args.window_size.clone() {
            if size.len() == 1 {
                window_size = Some((size[0], size[0]));
            } else if size.len() == 2 {
                window_size = Some((size[0], size[1]));
            }
        }
        let mut idle_browser_timeout = Duration::from_secs(600);
        if let Some(timeout) = args.idle_browser_timeout.clone() {
            idle_browser_timeout = Duration::from_secs(timeout as u64);
        }

        let browser: Browser;
        if is_connection {
            let debug_ws_url: String = args.chrome_path.clone().unwrap();
            browser = Browser::connect_with_timeout(debug_ws_url, idle_browser_timeout).map_err(|e| e.to_string())?;
        } else {
            let mut path: Option<PathBuf> = None;
            if let Some(chrome_path) = args.chrome_path.clone() {
                path = Some(PathBuf::from(chrome_path));
            }
            let launch_options = LaunchOptions {
                path,
                window_size,
                idle_browser_timeout,
                ..Default::default()
            };
            browser = Browser::new(launch_options).map_err(|e| e.to_string())?;
        }
        let tab = browser.new_tab().map_err(|e| e.to_string())?;
        let command_session: Box<dyn IntegrationSession> = Box::new(ChromeSession { browser, tab });
        gcx.write().await.integration_sessions.insert(
            session_hashmap_key.clone(), Arc::new(AMutex::new(command_session))
        );
    }
    Ok(true)
}


fn tool_message(content: String, tool_call_id: &String) -> ContextEnum {
    ContextEnum::ChatMessage(ChatMessage {
        role: "tool".to_string(),
        content: ChatContent::SimpleText(content.clone()),
        tool_calls: None,
        tool_call_id: tool_call_id.clone(),
        ..Default::default()
    })
}


async fn interact_with_chrome(
    command_args: &Vec<String>,
    session_hashmap_key: &String,
    tool_call_id: &String,
    gcx: Arc<ARwLock<GlobalContext>>) -> Result<Vec<ContextEnum>, String>
{
    let command_session = {
        let gcx_locked = gcx.read().await;
        gcx_locked.integration_sessions.get(session_hashmap_key)
            .ok_or(format!("Error getting chrome session for chat: {}", session_hashmap_key))?
            .clone()
    };

    let mut command_session_locked = command_session.lock().await;
    let chrome_session = command_session_locked.as_any_mut().downcast_mut::<ChromeSession>().ok_or("Failed to downcast to ChromeSession")?;

    let mut messages = vec![];
    if command_args[0] == "navigate_to" {
        if command_args.len() < 2 {
            messages.push(tool_message(format!("Missing argument `url`: {:?}", command_args), tool_call_id));
        } else {
            chrome_session.tab.navigate_to(command_args[1].as_str()).map_err(|e| e.to_string())?;
            chrome_session.tab.wait_until_navigated().map_err(|e| e.to_string())?;
            messages.push(tool_message(format!("Chrome tab navigated to {}", command_args[1]), tool_call_id));
        }
    } else if command_args[0] == "screenshot" {
        messages.push(tool_message("Made a screenshot".to_string(), tool_call_id));
        let screenshot_message = screenshot_jpeg_base64(&chrome_session.tab, false).await?;
        messages.push(ContextEnum::ChatMessage(screenshot_message));
    } else if command_args[0] == "html" {
        let content: String;
        let client = Client::builder()
            .build()
            .map_err(|e| e.to_string())?;
        let url = chrome_session.tab.get_url();
        let response = client.get(url.clone()).send().await.map_err(|e| e.to_string())?;
        if !response.status().is_success() {
            content = format!("Unable to fetch url: {}; status: {}", url, response.status());
        } else {
            content = response.text().await.map_err(|e| e.to_string())?;
        }
        messages.push(tool_message(content, tool_call_id));
    } else if command_args[0] == "reload" {
        // TODO: how to collect logs using this?
        // let content = Arc::new(Mutex::new(format!("Page {} reloaded with following log\n", chrome_session.tab.get_url())));
        // let listener = chrome_session.tab.add_event_listener(Arc::new(move |event: &Event| {
        //     let mut locked_content = content.lock().unwrap();
        //     match event {
        //         Event::LogEntryAdded(evt) => {
        //             locked_content.push_str(format!("\n[{:?}] {:?}", evt.params.entry.level, evt.params.entry.text).as_str());
        //         }
        //         _ => {
        //             // TODO: we need to catch more event probably
        //         }
        //     }
        // })).map_err(|e| e.to_string())?;
        // chrome_session.tab.enable_log().map_err(|e| e.to_string())?;
        chrome_session.tab.reload(false, None).map_err(|e| e.to_string())?;
        // chrome_session.tab.disable_log().map_err(|e| e.to_string())?;
        // let _ = chrome_session.tab.remove_event_listener(&listener);
        // let _content = content.lock().unwrap().clone();
        let content = format!("Page {} reloaded with following log", chrome_session.tab.get_url());
        messages.push(tool_message(content, tool_call_id));
    } else {
        return Err(format!("Unknown command: {:?}", command_args));
    }

    Ok(messages)
}

async fn is_chrome_session_active(
    key: &String,
    gcx: Arc<ARwLock<GlobalContext>>,
) -> bool {
    let session = {
        let gcx_locked = gcx.read().await;
        gcx_locked.integration_sessions.get(key).cloned()
    };
    !session.is_none()
}

async fn screenshot_jpeg_base64(tab: &Arc<Tab>, capture_beyond_viewport: bool) -> Result<ChatMessage, String> {
    let jpeg_data = tab.call_method(Page::CaptureScreenshot {
        format: Some(Page::CaptureScreenshotFormatOption::Jpeg),
        clip: None,
        quality: Some(75),
        from_surface: Some(true),
        capture_beyond_viewport: Some(capture_beyond_viewport),
    }).map_err(|e| e.to_string())?.data;

    let multimodal_element = MultimodalElement::new(
        "image/jpeg".to_string(), jpeg_data,
    ).map_err(|e| e.to_string())?;

    Ok(ChatMessage {
        role: "user".to_string(),  // Image URLs are only allowed for messages with role 'user'
        content: ChatContent::Multimodal(vec![multimodal_element]),
        ..Default::default()
    })
}
