use std::any::Any;
use std::sync::{Arc, Mutex};
use std::collections::HashMap;
use std::future::Future;
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
use crate::postprocessing::pp_command_output::{CmdlineOutputFilter, output_mini_postprocessing};
use crate::tools::tools_description::{Tool, ToolDesc, ToolParam};
use crate::integrations::integr_abstract::IntegrationTrait;

use tokio::time::sleep;
use chrono::DateTime;
use reqwest::Client;
use std::path::PathBuf;
use headless_chrome::{Browser, LaunchOptions, Tab as HeadlessTab};
use headless_chrome::browser::tab::point::Point;
use headless_chrome::protocol::cdp::Page;
use headless_chrome::protocol::cdp::Emulation;
use headless_chrome::protocol::cdp::types::Event;
use headless_chrome::protocol::cdp::DOM::Enable as DOMEnable;
use headless_chrome::protocol::cdp::CSS::Enable as CSSEnable;
use serde::{Deserialize, Serialize};

use base64::Engine;
use std::io::Cursor;
use image::imageops::FilterType;
use image::{ImageFormat, ImageReader};


#[derive(Clone, Serialize, Deserialize, Debug, Default)]
pub struct SettingsChrome {
    pub chrome_path: String,
    #[serde(default )]
    pub window_width: String,
    #[serde(default)]
    pub window_height: String,
    #[serde(default)]
    pub idle_browser_timeout: String,
    #[serde(default)]
    pub headless: String,
}

#[derive(Debug, Default)]
pub struct ToolChrome {
    pub settings_chrome: SettingsChrome,
    pub supports_clicks: bool,
}

#[derive(Clone, Debug)]
enum DeviceType {
    DESKTOP,
    MOBILE,
}

impl std::fmt::Display for DeviceType {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            DeviceType::DESKTOP => write!(f, "desktop"),
            DeviceType::MOBILE => write!(f, "mobile"),
        }
    }
}

const MAX_CACHED_LOG_LINES: usize = 1000;

#[derive(Clone)]
pub struct ChromeTab {
    headless_tab: Arc<HeadlessTab>,
    device: DeviceType,
    tab_id: String,
    screenshot_scale_factor: f64,
    tab_log: Arc<Mutex<Vec<String>>>,
}

impl ChromeTab {
    fn new(headless_tab: Arc<HeadlessTab>, device: &DeviceType, tab_id: &String) -> Self {
        Self {
            headless_tab,
            device: device.clone(),
            tab_id: tab_id.clone(),
            screenshot_scale_factor: 1.0,
            tab_log: Arc::new(Mutex::new(Vec::new())),
        }
    }
    pub fn state_string(&self) -> String {
        format!("tab_id `{}` device `{}` uri `{}`", self.tab_id.clone(), self.device, self.headless_tab.get_url())
    }
}

struct ChromeSession {
    browser: Browser,
    tabs: HashMap<String, Arc<AMutex<ChromeTab>>>,
}

impl ChromeSession {
    fn is_connected(&self) -> bool {
        match self.browser.get_version() {
            Ok(_) => {
                true
            },
            Err(_) => {
                false
            }
        }
    }
}

impl IntegrationSession for ChromeSession
{
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
    fn is_expired(&self) -> bool { false }
    fn try_stop(&mut self) -> Box<dyn Future<Output = String> + Send + '_> {
        Box::new(async { "".to_string() })
    }
}

impl IntegrationTrait for ToolChrome {
    fn as_any(&self) -> &dyn std::any::Any { self }

    fn integr_settings_apply(&mut self, value: &Value) -> Result<(), String> {
        match serde_json::from_value::<SettingsChrome>(value.clone()) {
            Ok(settings_chrome) => self.settings_chrome = settings_chrome,
            Err(e) => {
                tracing::error!("Failed to apply settings: {}\n{:?}", e, value);
                return Err(e.to_string());
            }
        }
        Ok(())
    }

    fn integr_settings_as_json(&self) -> Value {
        serde_json::to_value(&self.settings_chrome).unwrap()
    }

    fn integr_upgrade_to_tool(&self, _integr_name: &str) -> Box<dyn Tool + Send> {
        Box::new(ToolChrome {
            settings_chrome: self.settings_chrome.clone(),
            supports_clicks: false,
        }) as Box<dyn Tool + Send>
    }

    fn integr_schema(&self) -> &str
    {
        CHROME_INTEGRATION_SCHEMA
    }
}

#[async_trait]
impl Tool for ToolChrome {
    fn as_any(&self) -> &dyn std::any::Any { self }

    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let (gcx, chat_id) = {
            let ccx_lock = ccx.lock().await;
            (ccx_lock.global_context.clone(), ccx_lock.chat_id.clone())
        };

        let commands_str = match args.get("commands") {
            Some(Value::String(s)) => s,
            Some(v) => return Err(format!("argument `commands` is not a string: {:?}", v)),
            None => return Err("Missing argument `commands`".to_string())
        };

        let session_hashmap_key = get_session_hashmap_key("chrome", &chat_id);
        let mut tool_log = setup_chrome_session(gcx.clone(), &self.settings_chrome, &session_hashmap_key).await?;

        let command_session = {
            let gcx_locked = gcx.read().await;
            gcx_locked.integration_sessions.get(&session_hashmap_key)
                .ok_or(format!("Error getting chrome session for chat: {}", chat_id))?
                .clone()
        };

        let mut mutlimodal_els = vec![];
        for command in commands_str.lines().map(|s| s.trim()).collect::<Vec<&str>>() {
            let parsed_command = match parse_single_command(&command.to_string()) {
                Ok(command) => command,
                Err(e) => {
                    tool_log.push(format!("Failed to parse command `{}`: {}.", command, e));
                    break
                }
            };
            match chrome_command_exec(&parsed_command, command_session.clone()).await {
                Ok((execute_log, command_multimodal_els)) => {
                    tool_log.extend(execute_log);
                    mutlimodal_els.extend(command_multimodal_els);
                },
                Err(e) => {
                    tool_log.push(format!("Failed to execute command `{}`: {}.", command, e));
                    break
                }
            };
        }

        let mut content= vec![];
        content.push(MultimodalElement::new(
            "text".to_string(), tool_log.join("\n")
        )?);
        content.extend(mutlimodal_els);

        let msg = ContextEnum::ChatMessage(ChatMessage {
            role: "tool".to_string(),
            content: ChatContent::Multimodal(content),
            tool_calls: None,
            tool_call_id: tool_call_id.clone(),
            ..Default::default()
        });

        Ok((false, vec![msg]))
    }

    fn tool_description(&self) -> ToolDesc {
        let mut supported_commands = vec![
            "open_tab <tab_id> <desktop|mobile>",
            "navigate_to <tab_id> <uri>",
            "screenshot <tab_id>",
            // "html <tab_id>",
            "reload <tab_id>",
            "press_key_at <tab_id> <enter|esc|pageup|pagedown|home|end>",
            "type_text_at <tab_id> <text>",
            "tab_log <tab_id>",
            "eval <tab_id> <expression>",
            "styles <tab_id> <element_selector> <property_filter>",
            "click_at_element <tab_id> <element_selector>",
        ];
        if self.supports_clicks {
            supported_commands.extend(vec![
                "click_at_point <tab_id> <x> <y>",
            ]);
        }
        let description = format!(
            "One or several commands separated by newline. \
             The <tab_id> is an integer, for example 10, for you to identify the tab later. \
             Supported commands:\n{}", supported_commands.join("\n"));
        ToolDesc {
            name: "chrome".to_string(),
            agentic: true,
            experimental: true,
            description: "A real web browser with graphical interface.".to_string(),
            parameters: vec![ToolParam {
                name: "commands".to_string(),
                param_type: "string".to_string(),
                description,
            }],
            parameters_required: vec!["commands".to_string()],
        }
    }
}

async fn setup_chrome_session(
    gcx: Arc<ARwLock<GlobalContext>>,
    args: &SettingsChrome,
    session_hashmap_key: &String,
) -> Result<Vec<String>, String> {
    let mut setup_log = vec![];

    let session_entry  = {
        let gcx_locked = gcx.read().await;
        gcx_locked.integration_sessions.get(session_hashmap_key).cloned()
    };

    if let Some(session) = session_entry {
        let mut session_locked = session.lock().await;
        let chrome_session = session_locked.as_any_mut().downcast_mut::<ChromeSession>().ok_or("Failed to downcast to ChromeSession")?;
        if chrome_session.is_connected() {
            return Ok(setup_log)
        } else {
            setup_log.push("Chrome session is disconnected. Trying to reconnect.".to_string());
            gcx.write().await.integration_sessions.remove(session_hashmap_key);
        }
    }

    let window_size = match (args.window_width.parse::<u32>(), args.window_height.parse::<u32>()) {
        (Ok(width), Ok(height)) => Some((width, height)),
        _ => None,
    };

    let idle_browser_timeout = args.idle_browser_timeout
        .parse::<u64>()
        .map(Duration::from_secs)
        .unwrap_or(Duration::from_secs(600));

    let browser = if args.chrome_path.clone().starts_with("ws://") {
        let debug_ws_url: String = args.chrome_path.clone();
        setup_log.push("Connect to existing web socket.".to_string());
        Browser::connect_with_timeout(debug_ws_url, idle_browser_timeout).map_err(|e| e.to_string())
    } else {

        // let path = PathBuf::from(args.chrome_path.clone());
        let mut path: Option<PathBuf> = None;
        if !args.chrome_path.is_empty() {
            path = Some(PathBuf::from(args.chrome_path.clone()));
        }
        let launch_options = LaunchOptions {
            path,
            window_size,
            idle_browser_timeout,
            headless: args.headless.parse::<bool>().unwrap_or(true),
            ..Default::default()
        };
       
        setup_log.push("Started new chrome process.".to_string());
        Browser::new(launch_options).map_err(|e| e.to_string())
    }?;

    // NOTE: we're not register any tabs because they can be used by another chat
    setup_log.push("No opened tabs at this moment.".to_string());

    let command_session: Box<dyn IntegrationSession> = Box::new(ChromeSession { browser, tabs: HashMap::new() });
    gcx.write().await.integration_sessions.insert(
        session_hashmap_key.clone(), Arc::new(AMutex::new(command_session))
    );
    Ok(setup_log)
}

async fn screenshot_jpeg_base64(
    tab: Arc<AMutex<ChromeTab>>,
    capture_beyond_viewport: bool,
) -> Result<MultimodalElement, String> {
    let jpeg_base64_data = {
        let tab_lock = tab.lock().await;
        tab_lock.headless_tab.call_method(Page::CaptureScreenshot {
            format: Some(Page::CaptureScreenshotFormatOption::Jpeg),
            clip: None,
            quality: Some(75),
            from_surface: Some(true),
            capture_beyond_viewport: Some(capture_beyond_viewport),
        }).map_err(|e| e.to_string())?.data
    };

    let mut data = base64::prelude::BASE64_STANDARD
        .decode(jpeg_base64_data).map_err(|e| e.to_string())?;
    let reader = ImageReader::with_format(Cursor::new(data), ImageFormat::Jpeg);
    let mut image = reader.decode().map_err(|e| e.to_string())?;

    let max_dimension = 800.0;
    let scale_factor = max_dimension / std::cmp::max(image.width(), image.height()) as f32;
    if scale_factor < 1.0 {
        // NOTE: the tool operates on resized image well without a special model notification
        let (nwidth, nheight) = (scale_factor * image.width() as f32, scale_factor * image.height() as f32);
        image = image.resize(nwidth as u32, nheight as u32, FilterType::Lanczos3);
        // NOTE: we should store screenshot_scale_factor for every resized screenshot, not for a tab!
        let mut tab_lock = tab.lock().await;
        tab_lock.screenshot_scale_factor = scale_factor as f64;
    }

    data = Vec::new();
    image.write_to(&mut Cursor::new(&mut data), ImageFormat::Jpeg).map_err(|e| e.to_string())?;

    MultimodalElement::new("image/jpeg".to_string(), base64::prelude::BASE64_STANDARD.encode(data))
}

async fn session_open_tab(
    chrome_session: &mut ChromeSession,
    tab_id: &String,
    device: &DeviceType,
) -> Result<String, String> {
    match chrome_session.tabs.get(tab_id) {
        Some(tab) => {
            let tab_lock = tab.lock().await;
            Err(format!("Tab is already opened: {}\n", tab_lock.state_string()))
        },
        None => {
            let headless_tab = chrome_session.browser.new_tab().map_err(|e| e.to_string())?;
            match device {
                DeviceType::MOBILE => {
                    headless_tab.call_method(Emulation::SetDeviceMetricsOverride {
                        width: 375,
                        height: 812,
                        device_scale_factor: 0.0,
                        mobile: true,
                        scale: None,
                        screen_width: None,
                        screen_height: None,
                        position_x: None,
                        position_y: None,
                        dont_set_visible_size: None,
                        screen_orientation: None,
                        viewport: None,
                        display_feature: None,
                    }).map_err(|e| e.to_string())?;
                },
                DeviceType::DESKTOP => {
                    headless_tab.call_method(Emulation::ClearDeviceMetricsOverride(None)).map_err(|e| e.to_string())?;
                }
            }
            let tab = Arc::new(AMutex::new(ChromeTab::new(headless_tab, device, tab_id)));
            let tab_lock = tab.lock().await;
            let tab_log = Arc::clone(&tab_lock.tab_log);
            tab_lock.headless_tab.enable_log().map_err(|e| e.to_string())?;
            tab_lock.headless_tab.add_event_listener(Arc::new(move |event: &Event| {
                if let Event::LogEntryAdded(e) = event {
                    let formatted_ts = {
                        let dt = DateTime::from_timestamp(e.params.entry.timestamp as i64, 0).unwrap();
                        dt.format("%Y-%m-%d %H:%M:%S").to_string()
                    };
                    let mut tab_log_lock = tab_log.lock().unwrap();
                    tab_log_lock.push(format!("{} [{:?}]: {}", formatted_ts, e.params.entry.level, e.params.entry.text));
                    if tab_log_lock.len() > MAX_CACHED_LOG_LINES {
                        tab_log_lock.remove(0);
                    }
                }
            })).map_err(|e| e.to_string())?;
            chrome_session.tabs.insert(tab_id.clone(), tab.clone());
            Ok(format!("Opened a new tab: {}\n", tab_lock.state_string()))
        }
    }
}

async fn session_get_tab_arc(
    chrome_session: &ChromeSession,
    tab_id: &String,
) -> Result<Arc<AMutex<ChromeTab>>, String> {
    match chrome_session.tabs.get(tab_id) {
        Some(tab) => Ok(tab.clone()),
        None => Err(format!("tab_id {} is not opened", tab_id)),
    }
}

#[derive(Debug)]
enum Command {
    OpenTab(OpenTabArgs),
    NavigateTo(NavigateToArgs),
    Screenshot(TabArgs),
    Html(TabArgs),
    Reload(TabArgs),
    ClickAtPoint(ClickAtPointArgs),
    ClickAtElement(TabElementArgs),
    TypeTextAt(TypeTextAtArgs),
    PressKeyAt(PressKeyAtArgs),
    TabLog(TabArgs),
    Eval(EvalArgs),
    Styles(StylesArgs),
}

async fn chrome_command_exec(
    cmd: &Command,
    chrome_session: Arc<AMutex<Box<dyn IntegrationSession>>>,
) -> Result<(Vec<String>, Vec<MultimodalElement>), String> {
    let mut tool_log = vec![];
    let mut multimodal_els = vec![];

    match cmd {
        Command::OpenTab(args) => {
            let log = {
                let mut chrome_session_locked = chrome_session.lock().await;
                let chrome_session = chrome_session_locked.as_any_mut().downcast_mut::<ChromeSession>().ok_or("Failed to downcast to ChromeSession")?;
                session_open_tab(chrome_session, &args.tab_id, &args.device).await?
            };
            tool_log.push(log);
        },
        Command::NavigateTo(args) => {
            let tab: Arc<AMutex<ChromeTab>> = {
                let mut chrome_session_locked = chrome_session.lock().await;
                let chrome_session = chrome_session_locked.as_any_mut().downcast_mut::<ChromeSession>().ok_or("Failed to downcast to ChromeSession")?;
                session_get_tab_arc(chrome_session, &args.tab_id).await?
            };
            let log = {
                let tab_lock = tab.lock().await;
                match {
                    tab_lock.headless_tab.navigate_to(args.uri.as_str()).map_err(|e| e.to_string())?;
                    tab_lock.headless_tab.wait_until_navigated().map_err(|e| e.to_string())?;
                    Ok::<(), String>(())
                } {
                    Ok(_) => {
                        format!("navigate_to successful: {}", tab_lock.state_string())
                    },
                    Err(e) => {
                        format!("navigate_to `{}` failed: {}. If you're trying to open a local file, add a file:// prefix.", args.uri, e.to_string())
                    },
                }
            };
            tool_log.push(log);
        },
        Command::Screenshot(args) => {
            let tab = {
                let mut chrome_session_locked = chrome_session.lock().await;
                let chrome_session = chrome_session_locked.as_any_mut().downcast_mut::<ChromeSession>().ok_or("Failed to downcast to ChromeSession")?;
                session_get_tab_arc(chrome_session, &args.tab_id).await?
            };
            let log = {
                // NOTE: this operation is not atomic, unfortunately
                match screenshot_jpeg_base64(tab.clone(), false).await {
                    Ok(multimodal_el) => {
                        multimodal_els.push(multimodal_el);
                        let tab_lock = tab.lock().await;
                        format!("Made a screenshot of {}", tab_lock.state_string())
                    },
                    Err(e) => {
                        let tab_lock = tab.lock().await;
                        format!("Screenshot failed for {}: {}", tab_lock.state_string(), e.to_string())
                    },
                }
            };
            tool_log.push(log);
        },
        Command::Html(args) => {
            // NOTE: removed from commands list, please rewrite me...
            let tab = {
                let mut chrome_session_locked = chrome_session.lock().await;
                let chrome_session = chrome_session_locked.as_any_mut().downcast_mut::<ChromeSession>().ok_or("Failed to downcast to ChromeSession")?;
                session_get_tab_arc(chrome_session, &args.tab_id).await?
            };
            let log = {
                let tab_lock = tab.lock().await;
                let url = tab_lock.headless_tab.get_url();
                match {
                    let client = Client::builder()
                        .build()
                        .map_err(|e| e.to_string())?;
                    let response = client.get(url.clone()).send().await.map_err(|e| e.to_string())?;
                    if response.status().is_success() {
                        let html = response.text().await.map_err(|e| e.to_string())?;
                        Ok(html)
                    } else {
                        Err(format!("status: {}", response.status()))
                    }
                } {
                    Ok(html) => {
                        format!("innerHtml of {}:\n\n{}", tab_lock.state_string(), html)
                    },
                    Err(e) => {
                        format!("can't fetch innerHtml of {}: {}", tab_lock.state_string(), e.to_string())
                    },
                }
            };
            tool_log.push(log);
        },
        Command::Reload(args) => {
            let tab = {
                let mut chrome_session_locked = chrome_session.lock().await;
                let chrome_session = chrome_session_locked.as_any_mut().downcast_mut::<ChromeSession>().ok_or("Failed to downcast to ChromeSession")?;
                session_get_tab_arc(chrome_session, &args.tab_id).await?
            };
            let log = {
                let tab_lock = tab.lock().await;
                let chrome_tab = tab_lock.headless_tab.clone();
                match chrome_tab.reload(false, None) {
                    Ok(_) => {
                        format!("reload of {} successful", tab_lock.state_string())
                    },
                    Err(e) => {
                        format!("reload of {} failed: {}", tab_lock.state_string(), e.to_string())
                    },
                }
            };
            tool_log.push(log);
        },
        Command::ClickAtPoint(args) => {
            let tab = {
                let mut chrome_session_locked = chrome_session.lock().await;
                let chrome_session = chrome_session_locked.as_any_mut().downcast_mut::<ChromeSession>().ok_or("Failed to downcast to ChromeSession")?;
                session_get_tab_arc(chrome_session, &args.tab_id).await?
            };
            let log = {
                let tab_lock = tab.lock().await;
                match {
                    let mapped_point = Point {
                        x: args.point.x / tab_lock.screenshot_scale_factor,
                        y: args.point.y / tab_lock.screenshot_scale_factor,
                    };
                    tab_lock.headless_tab.click_point(mapped_point).map_err(|e| e.to_string())?;
                    tab_lock.headless_tab.wait_until_navigated().map_err(|e| e.to_string())?;
                    Ok::<(), String>(())
                } {
                    Ok(_) => {
                        format!("clicked `{} {}` at {}", args.point.x, args.point.y, tab_lock.state_string())
                    },
                    Err(e) => {
                        format!("clicked `{} {}` failed at {}: {}", args.point.x, args.point.y, tab_lock.state_string(), e.to_string())
                    },
                }
            };
            tool_log.push(log);
        },
        Command::ClickAtElement(args) => {
            let tab = {
                let mut chrome_session_locked = chrome_session.lock().await;
                let chrome_session = chrome_session_locked.as_any_mut().downcast_mut::<ChromeSession>().ok_or("Failed to downcast to ChromeSession")?;
                session_get_tab_arc(chrome_session, &args.tab_id).await?
            };
            let log = {
                let tab_lock = tab.lock().await;
                match {
                    let element = tab_lock.headless_tab.find_element(&args.selector).map_err(|e| e.to_string())?;
                    element.click().map_err(|e| e.to_string())?;
                    Ok::<(), String>(())
                } {
                    Ok(_) => {
                        format!("clicked `{}` at {}", args.selector, tab_lock.state_string())
                    },
                    Err(e) => {
                        format!("click at element `{}` failed at {}: {}", args.selector, tab_lock.state_string(), e.to_string())
                    },
                }
            };
            tool_log.push(log);
        },
        Command::TypeTextAt(args) => {
            let tab = {
                let mut chrome_session_locked = chrome_session.lock().await;
                let chrome_session = chrome_session_locked.as_any_mut().downcast_mut::<ChromeSession>().ok_or("Failed to downcast to ChromeSession")?;
                session_get_tab_arc(chrome_session, &args.tab_id).await?
            };
            let log = {
                let tab_lock = tab.lock().await;
                match tab_lock.headless_tab.type_str(args.text.as_str()) {
                    Ok(_) => {
                        format!("type `{}` at {}", args.text, tab_lock.state_string())
                    },
                    Err(e) => {
                        format!("type text failed at {}: {}", tab_lock.state_string(), e.to_string())
                    },
                }
            };
            tool_log.push(log);
        },
        Command::PressKeyAt(args) => {
            let tab = {
                let mut chrome_session_locked = chrome_session.lock().await;
                let chrome_session = chrome_session_locked.as_any_mut().downcast_mut::<ChromeSession>().ok_or("Failed to downcast to ChromeSession")?;
                session_get_tab_arc(chrome_session, &args.tab_id).await?
            };
            let log = {
                let tab_lock = tab.lock().await;
                match {
                    tab_lock.headless_tab.press_key(args.key.to_string().as_str()).map_err(|e| e.to_string())?;
                    tab_lock.headless_tab.wait_until_navigated().map_err(|e| e.to_string())?;
                    // TODO: sometimes page isn't ready for next step
                    sleep(Duration::from_secs(1)).await;
                    Ok::<(), String>(())
                } {
                    Ok(_) => {
                        format!("press `{}` at {}", args.key, tab_lock.state_string())
                    },
                    Err(e) => {
                        format!("press `{}` failed at {}: {}", args.key, tab_lock.state_string(), e.to_string())
                    },
                }
            };
            tool_log.push(log);
        },
        Command::TabLog(args) => {
            let tab = {
                let mut chrome_session_locked = chrome_session.lock().await;
                let chrome_session = chrome_session_locked.as_any_mut().downcast_mut::<ChromeSession>().ok_or("Failed to downcast to ChromeSession")?;
                session_get_tab_arc(chrome_session, &args.tab_id).await?
            };
            let tab_log = {
                let tab_lock = tab.lock().await;
                // NOTE: we're waiting for log to be collected for 3 seconds
                sleep(Duration::from_secs(3)).await;
                let mut tab_log_lock = tab_lock.tab_log.lock().unwrap();
                let tab_log = tab_log_lock.join("\n");
                tab_log_lock.clear();
                tab_log
            };
            let filter = CmdlineOutputFilter::default();
            let filtered_log = output_mini_postprocessing(&filter, tab_log.as_str());
            tool_log.push(filtered_log.clone());
        },
        Command::Eval(args) => {
            let tab = {
                let mut chrome_session_locked = chrome_session.lock().await;
                let chrome_session = chrome_session_locked.as_any_mut().downcast_mut::<ChromeSession>().ok_or("Failed to downcast to ChromeSession")?;
                session_get_tab_arc(chrome_session, &args.tab_id).await?
            };
            let log = {
                let tab_lock = tab.lock().await;
                match tab_lock.headless_tab.evaluate(args.expression.as_str(), false) {
                    Ok(result) => {
                        format!("eval result at {}: {:?}", tab_lock.state_string(), result)
                    },
                    Err(e) => {
                        format!("eval failed at {}: {}", tab_lock.state_string(), e.to_string())
                    },
                }
            };
            tool_log.push(log);
        },
        Command::Styles(args) => {
            let tab = {
                let mut chrome_session_locked = chrome_session.lock().await;
                let chrome_session = chrome_session_locked.as_any_mut().downcast_mut::<ChromeSession>().ok_or("Failed to downcast to ChromeSession")?;
                session_get_tab_arc(chrome_session, &args.tab_id).await?
            };
            let log = {
                let tab_lock = tab.lock().await;
                match {
                    tab_lock.headless_tab.call_method(DOMEnable(None)).map_err(|e| e.to_string())?;
                    tab_lock.headless_tab.call_method(CSSEnable(None)).map_err(|e| e.to_string())?;
                    let element = tab_lock.headless_tab.find_element(&args.selector).map_err(|e| e.to_string())?;
                    let computed_styles = element.get_computed_styles().map_err(|e| e.to_string())?;
                    let mut styles_filtered = computed_styles.iter()
                        .filter(|s| s.name.contains(args.property_filter.as_str()))
                        .map(|s| format!("{}: {}", s.name, s.value))
                        .collect::<Vec<String>>();
                    let max_lines_output = 30;
                    if styles_filtered.len() > max_lines_output {
                        let skipped_message = format!("Skipped {} properties. Specify filter if you need to see more.", styles_filtered.len() - max_lines_output);
                        styles_filtered = styles_filtered[..max_lines_output].to_vec();
                        styles_filtered.push(skipped_message)
                    }
                    if styles_filtered.is_empty() {
                        styles_filtered.push("No properties for given filter.".to_string());
                    }
                    Ok::<String, String>(styles_filtered.join("\n"))
                } {
                    Ok(styles_str) => {
                        format!("Style properties for element `{}` at {}:\n{}", args.selector, tab_lock.state_string(), styles_str)
                    },
                    Err(e) => {
                        format!("Styles get failed at {}: {}", tab_lock.state_string(), e.to_string())
                    },
                }
            };
            tool_log.push(log);
        },
    }

    Ok((tool_log, multimodal_els))
}

#[derive(Debug)]
struct TabArgs {
    tab_id: String,
}

#[derive(Debug)]
struct OpenTabArgs {
    device: DeviceType,
    tab_id: String,
}

#[derive(Debug)]
struct NavigateToArgs {
    uri: String,
    tab_id: String,
}

#[derive(Debug)]
struct ClickAtPointArgs {
    point: Point,
    tab_id: String,
}

#[derive(Debug)]
struct TypeTextAtArgs {
    text: String,
    tab_id: String,
}

#[derive(Clone, Debug)]
enum Key {
    ENTER,
    ESC,
    PAGEUP,
    PAGEDOWN,
    HOME,
    END,
}

impl std::fmt::Display for Key {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Key::ENTER => write!(f, "Enter"),
            Key::ESC => write!(f, "Escape"),
            Key::PAGEUP => write!(f, "PageUp"),
            Key::PAGEDOWN => write!(f, "PageDown"),
            Key::HOME => write!(f, "Home"),
            Key::END => write!(f, "End"),
        }
    }
}

#[derive(Debug)]
struct PressKeyAtArgs {
    key: Key,
    tab_id: String,
}

#[derive(Debug)]
struct EvalArgs {
    tab_id: String,
    expression: String,
}

#[derive(Debug)]
struct TabElementArgs {
    tab_id: String,
    selector: String,
}

#[derive(Debug)]
struct StylesArgs {
    tab_id: String,
    selector: String,
    property_filter: String,
}

fn parse_single_command(command: &String) -> Result<Command, String> {
    let args = shell_words::split(&command).map_err(|e| e.to_string())?;
    if args.is_empty() {
        return Err("Command is empty".to_string());
    }

    let (command_name, parsed_args) = (args[0].clone(), args[1..].to_vec());

    match command_name.as_str() {
        "open_tab" => {
            match parsed_args.as_slice() {
                [tab_id, device_str] => {
                    let device = match device_str.as_str() {
                        "desktop" => DeviceType::DESKTOP,
                        "mobile" => DeviceType::MOBILE,
                        _ => return Err(format!("unknown device type: {}. Should be either `desktop` or `mobile`.", parsed_args[0]))
                    };
                    Ok(Command::OpenTab(OpenTabArgs {
                        device: device.clone(),
                        tab_id: tab_id.clone(),
                    }))
                },
                _ => {
                    Err("Missing one or several arguments `tab_id`, `<device|mobile>`".to_string())
                }
            }
        },
        "navigate_to" => {
            match parsed_args.as_slice() {
                [tab_id, uri] => {
                    Ok(Command::NavigateTo(NavigateToArgs {
                        uri: uri.clone(),
                        tab_id: tab_id.clone(),
                    }))
                },
                _ => {
                    Err("Missing one or several arguments `tab_id`, `uri`".to_string())
                }
            }
        },
        "screenshot" => {
            match parsed_args.as_slice() {
                [tab_id] => {
                    Ok(Command::Screenshot(TabArgs {
                        tab_id: tab_id.clone(),
                    }))
                },
                _ => {
                    Err("Missing one or several arguments `tab_id`".to_string())
                }
            }
        },
        "html" => {
            match parsed_args.as_slice() {
                [tab_id] => {
                    Ok(Command::Html(TabArgs {
                        tab_id: tab_id.clone(),
                    }))
                },
                _ => {
                    Err("Missing one or several arguments `tab_id`".to_string())
                }
            }
        },
        "reload" => {
            match parsed_args.as_slice() {
                [tab_id] => {
                    Ok(Command::Reload(TabArgs {
                        tab_id: tab_id.clone(),
                    }))
                },
                _ => {
                    Err("Missing one or several arguments `tab_id`".to_string())
                }
            }
        },
        "click_at_point" => {
            match parsed_args.as_slice() {
                [tab_id, x_str, y_str] => {
                    let x = x_str.parse::<f64>().map_err(|e| format!("Failed to parse x: {}", e))?;
                    let y = y_str.parse::<f64>().map_err(|e| format!("Failed to parse y: {}", e))?;
                    let point = Point { x, y };
                    Ok(Command::ClickAtPoint(ClickAtPointArgs {
                        point,
                        tab_id: tab_id.clone(),
                    }))
                },
                _ => {
                    Err("Missing one or several arguments `tab_id`, `x`, 'y`".to_string())
                }
            }
        },
        "click_at_element" => {
            match parsed_args.as_slice() {
                [tab_id, selector] => {
                    Ok(Command::ClickAtElement(TabElementArgs {
                        selector: selector.clone(),
                        tab_id: tab_id.clone(),
                    }))
                },
                _ => {
                    Err("Missing one or several arguments `tab_id`, `selector`".to_string())
                }
            }
        },
        "type_text_at" => {
            match parsed_args.as_slice() {
                [tab_id, text] => {
                    Ok(Command::TypeTextAt(TypeTextAtArgs {
                        text: text.clone(),
                        tab_id: tab_id.clone(),
                    }))
                },
                _ => {
                    Err("Missing one or several arguments `tab_id`, `text`".to_string())
                }
            }
        },
        "press_key_at" => {
            match parsed_args.as_slice() {
                [tab_id, key_str] => {
                    let key = match key_str.to_lowercase().as_str() {
                        "enter" => Key::ENTER,
                        "esc" => Key::ESC,
                        "pageup" => Key::PAGEUP,
                        "pagedown" => Key::PAGEDOWN,
                        "home" => Key::HOME,
                        "end" => Key::END,
                        _ => return Err(format!("Unknown key: {}", key_str)),
                    };
                    Ok(Command::PressKeyAt(PressKeyAtArgs {
                        key,
                        tab_id: tab_id.clone(),
                    }))
                },
                _ => {
                    Err("Missing one or several arguments `tab_id`, `key`".to_string())
                }
            }
        },
        "tab_log" => {
            match parsed_args.as_slice() {
                [tab_id] => {
                    Ok(Command::TabLog(TabArgs {
                        tab_id: tab_id.clone(),
                    }))
                },
                _ => {
                    Err("Missing one or several arguments `tab_id`".to_string())
                }
            }
        },
        "eval" => {
            match parsed_args.as_slice() {
                [tab_id, expression] => {
                    Ok(Command::Eval(EvalArgs {
                        expression: expression.clone(),
                        tab_id: tab_id.clone(),
                    }))
                },
                _ => {
                    Err("Missing one or several arguments `tab_id`, `expression`.".to_string())
                }
            }
        },
        "styles" => {
            match parsed_args.as_slice() {
                [tab_id, selector, property_filter] => {
                    Ok(Command::Styles(StylesArgs {
                        selector: selector.clone(),
                        tab_id: tab_id.clone(),
                        property_filter: property_filter.clone(),
                    }))
                },
                _ => {
                    Err("Missing one or several arguments `tab_id`, `selector`.".to_string())
                }
            }
        },
        _ => Err(format!("Unknown command: {:?}.", command_name)),
    }
}

const CHROME_INTEGRATION_SCHEMA: &str = r#"
fields:
  chrome_path:
    f_type: string_long
    f_desc: "Path to Google Chrome or Chromium binary. If empty, it searches for Google Chrome in your system"
    f_placeholder: ""
  window_width:
    f_type: string_short
    f_desc: "Width of the browser window."
    f_default: ""
    f_extra: true
  window_height:
    f_type: string_short
    f_desc: "Height of the browser window."
    f_default: ""
    f_extra: true
  idle_browser_timeout:
    f_type: string_short
    f_desc: "Idle timeout for the browser in seconds."
    f_default: ""
    f_extra: true
  headless:
    f_type: string_short
    f_desc: "Run Chrome in headless mode."
    f_default: "true"
    f_extra: true
available:
  on_your_laptop_possible: true
  when_isolated_possible: true
smartlinks:
  - sl_label: "Test"
    sl_chat:
      - role: "user"
        content: |
          🔧 The chrome tool should be visible now. To test the tool, navigate to a website like https://example.com/ take a screenshot, and express happiness if it works. If it doesn't work or the tool isn't available, go through the usual plan in the system prompt.
  - sl_label: "Help me install Chrome for Testing"
    sl_chat:
      - role: "user"
        content: |
          🔧 Help user to install Chrome for Testing using npm, once that done rewrite the current config file %CURRENT_CONFIG% to use it.
  - sl_label: "Help me connect regular Chrome via ws:// protocol"
    sl_chat:
      - role: "user"
        content: |
          🔧 Help user to connect regular Chrome via ws:// protocol, rewrite the current config file %CURRENT_CONFIG% to use it. The `chrome_path` accepts the "ws://..." notation.
docker:
  filter_label: ""
  filter_image: "standalone-chrome"
  new_container_default:
    image: "selenium/standalone-chrome:latest"
    environment: {}
  smartlinks:
    - sl_label: "Add Chrome Container"
      sl_chat:
        - role: "user"
          content: |
            🔧 Your job is to create a chrome container, using the image and environment from new_container_default section in the current config file: %CURRENT_CONFIG%. Follow the system prompt.
  smartlinks_for_each_container:
    - sl_label: "Use for integration"
      sl_chat:
        - role: "user"
          content: |
            🔧 Your job is to modify chrome config in the current file to connect through websockets to the container, use docker tool to inspect the container if needed. Current config file: %CURRENT_CONFIG%.
"#;