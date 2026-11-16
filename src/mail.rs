//! Mailers ported from `app/mailers`, with the multipart layout and subjects.
//!
//! Bodies are rendered here (text + HTML) so both the Sidekiq job consumer and
//! the harness can compare them; delivery goes through SMTP when
//! `SMTP_ADDRESS` is configured, otherwise through the local `sendmail`, which
//! is what `config/environments/production.rb` falls back to.

use crate::config::Config;
use crate::web;

pub struct Message {
    pub to: String,
    pub subject: String,
    pub text: String,
    pub html: String,
}

/// `ActionMailer::Base.default from: ENV["FROM_EMAIL"]`.
fn from_address() -> String {
    std::env::var("FROM_EMAIL")
        .ok()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "donotreply@example.com".to_owned())
}

// ------------------------------------------------------------------ layout

/// `layouts/mailer.html.erb`, with `yield` placed at the same indentation.
fn html_layout(cfg: &Config, to: &str, content: &str) -> String {
    let base = format!("{}/", cfg.base_url());
    let logo = web::image_url(cfg, "logo_email.jpg");
    let stylesheet = web::image_url(cfg, "mailers.css");
    format!(
        concat!(
            "<!DOCTYPE html PUBLIC \"-//W3C//DTD XHTML 1.0 Strict//EN\" ",
            "\"http://www.w3.org/TR/xhtml1/DTD/xhtml1-strict.dtd\">\n\n",
            "<html xmlns=\"http://www.w3.org/1999/xhtml\">\n<head>\n",
            "  <meta http-equiv=\"Content-Type\" content=\"text/html; charset=utf-8\" />\n",
            "  <meta name=\"viewport\" content=\"width=device-width, initial-scale=1.0\"/>\n",
            "  <title>Supermarket</title>\n",
            "  <link rel=\"stylesheet\" href=\"{stylesheet}\" media=\"all\" />\n",
            "</head>\n<body>\n",
            "  <table cellpadding=\"0\" cellspacing=\"0\" border=\"0\" width=\"100%\" id=\"wrapper\">\n",
            "    <tr>\n      <td align=\"center\">\n",
            "        <table class=\"content\" cellpadding=\"0\" cellspacing=\"0\" border=\"0\">\n",
            "          <tr>\n            <td class=\"header\" align=\"center\">\n",
            "              <a href=\"{base}\">\n",
            "                <img alt=\"Supermarket\" title=\"Supermarket\" width=\"251\" ",
            "height=\"63\" class=\"image_fix\" src=\"{logo}\" />\n",
            "</a>            </td>\n          </tr>\n          <tr>\n",
            "            <td class=\"body\">\n              {content}\n            </td>\n",
            "          </tr>\n        </table>\n",
            "        <p align=\"center\" class=\"footer\">\n",
            "          <strong>Supermarket</strong> &#8226; 619 Western Ave #400 &#8226; Seattle, WA 98104<br>\n",
            "          This email was sent to {to} by <a href=\"{base}\">Supermarket</a>.\n",
            "        </p>\n      </td>\n    </tr>\n  </table>\n",
            "  <!-- End of wrapper table -->\n</body>\n</html>\n"
        ),
        base = base,
        logo = logo,
        stylesheet = stylesheet,
        content = content,
        to = to,
    )
}

/// `layouts/mailer.text.erb`.
fn text_layout(cfg: &Config, to: &str, content: &str) -> String {
    format!(
        "{content}\n_________________________________________________________\n\n\
         Supermarket - 619 Western Ave. #400 - Seattle, WA 98104\n\
         This email was sent to {to} by Supermarket ({}).\n",
        root_url(cfg)
    )
}

fn root_url(cfg: &Config) -> String {
    format!("{}/", cfg.base_url())
}

/// `ERB::Util.html_escape` spelling for the five significant bytes.
fn escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            other => out.push(other),
        }
    }
    out
}

// ------------------------------------------------------------------ mailers

/// `CookbookMailer#follower_notification_email`.
#[allow(clippy::too_many_arguments)]
pub fn follower_notification(
    cfg: &Config,
    to: &str,
    cookbook: &str,
    version: &str,
    version_url: &str,
    changelog_html: Option<&str>,
    changelog_url: &str,
    unsubscribe: &str,
) -> Message {
    let changelog = match changelog_html {
        Some(html) => format!(
            "<div class=\"changelog\">\n  <h2 class=\"title\">Changelog</h2>\n  \
             {html}\n\n  <a class=\"button accept\" href=\"{changelog_url}\">\
             View Full Changelog</a>\n</div>",
        ),
        None => "<div class=\"changelog\">\n</div>".to_owned(),
    };
    let html = format!(
        "<h1>New version of {name} released.</h1>\n\
         <p>A new version of {name} has been released [{version}].</p>\n\
         <a class=\"button accept\" href=\"{version_url}\">View Version</a>\n\
         \n{changelog}\n\n<a href=\"{unsubscribe}\">Unsubscribe</a>\n",
        name = escape(cookbook),
        version = escape(version),
        version_url = version_url,
        changelog = changelog,
        unsubscribe = unsubscribe,
    );
    let mut text = format!(
        "Supermarket - New Cookbook Version\n\
         _________________________________________________________\n\n\n\
         New version of {name} released.\n\n\
         A new version of {name} has been released [{version}].\n\n\
         {version_url}\n\n",
        name = cookbook,
        version = version,
        version_url = version_url,
    );
    if let Some(html) = changelog_html {
        text.push_str(&format!(
            "\nChangelog\n_________________________________________________________\n\n\
             {}\n\n{}\n",
            truncate_html(html, 200),
            changelog_url,
        ));
    }
    text.push_str(&format!("\nUnsubscribe from this email: {unsubscribe}\n"));
    Message {
        to: to.to_owned(),
        subject: format!("A new version of the {cookbook} cookbook has been released"),
        text: text_layout(cfg, to, &text),
        html: html_layout(cfg, to, &html),
    }
}

/// `CookbookMailer#cookbook_deleted_email`.
pub fn cookbook_deleted(cfg: &Config, to: &str, name: &str, unsubscribe: &str) -> Message {
    let html = format!(
        "<p>The {name} cookbook, which you follow or are a collaborator on, has been deleted.</p>\n\n\
         <a href=\"{unsubscribe}\">Unsubscribe</a>\n",
        name = escape(name),
    );
    let text = format!(
        "The {name} cookbook, which you follow or are a collaborator on, has been deleted.\n\n\
         Unsubscribe from this email: {unsubscribe}\n"
    );
    Message {
        to: to.to_owned(),
        subject: format!("The {name} cookbook has been deleted"),
        text: text_layout(cfg, to, &text),
        html: html_layout(cfg, to, &html),
    }
}

/// `CookbookMailer#cookbook_deprecated_email`.
pub fn cookbook_deprecated(
    cfg: &Config,
    to: &str,
    name: &str,
    cookbook_url: &str,
    reason: &str,
    replacement: Option<(&str, &str)>,
    unsubscribe: &str,
) -> Message {
    let mut html = format!(
        "<p>The <a href=\"{cookbook_url}\">{name}</a> cookbook has been\n   deprecated.</p>\n\
         <p>Author provided reason for deprecation:</p>\n<p>{reason}</p>\n",
        name = escape(name),
        reason = escape(reason),
    );
    let mut subject = format!("The {name} cookbook has been deprecated");
    if let Some((replacement_name, replacement_url)) = replacement {
        html.push_str(&format!(
            "<p>The owner recommends using the <a href=\"{replacement_url}\">{replacement_name}</a>\n   cookbook.</p>\n",
            replacement_name = escape(replacement_name),
        ));
        subject.push_str(&format!(" in favor of the {replacement_name} cookbook"));
    }
    html.push_str(&format!("\n<a href=\"{unsubscribe}\">Unsubscribe</a>\n"));

    let mut text = format!(
        "The {name} cookbook [{cookbook_url}] has been deprecated.\n\
         <p>Author provided reason for deprecation:</p>\n<p>{reason}</p>\n"
    );
    if let Some((replacement_name, replacement_url)) = replacement {
        text.push_str(&format!(
            "The owner recommends using the {replacement_name} cookbook [ {replacement_url} ].\n"
        ));
    }
    text.push_str(&format!("\nUnsubscribe from this email: {unsubscribe}\n"));
    Message {
        to: to.to_owned(),
        subject,
        text: text_layout(cfg, to, &text),
        html: html_layout(cfg, to, &html),
    }
}

/// `CookbookMailer#transfer_ownership_email`.
pub fn transfer_ownership(
    cfg: &Config,
    to: &str,
    sender: &str,
    cookbook: &str,
    accept_url: &str,
    decline_url: &str,
) -> Message {
    let html = format!(
        "<h1>{sender} wants to transfer ownership of the {cookbook} cookbook to you.</h1>\n\n\
         <a class=\"button accept\" href=\"{accept_url}\">Accept</a>\n\
         <a class=\"button decline\" href=\"{decline_url}\">Decline</a>\n",
        sender = escape(sender),
        cookbook = escape(cookbook),
    );
    let text = format!(
        "{sender} wants to transfer ownership of the {cookbook} cookbook to you.\n\n\
         To accept, click here: {accept_url}\nTo decline, click here: {decline_url}\n"
    );
    Message {
        to: to.to_owned(),
        subject: format!(
            "{sender} wants to transfer ownership of the {cookbook} cookbook to you and add themselves as a collaborator."
        ),
        text: text_layout(cfg, "", &text),
        html: html_layout(cfg, "", &html),
    }
}

/// `CollaboratorMailer#added_email`.
pub fn collaborator_added(
    cfg: &Config,
    to: &str,
    resource: &str,
    class_name: &str,
    url: &str,
) -> Message {
    let html = format!(
        "<h1>Collaborator Notification</h1>\n\n\
         <p>You have been added as a collaborator to the <a href=\"{url}\">{resource}</a> {class_name}.</p>\n",
        resource = escape(resource),
        class_name = escape(class_name),
    );
    let text = format!(
        "Supermarket - Collaborator Notification\n\
         _________________________________________________________\n\n\
         You have been added as a collaborator to the {resource} {class_name}.  \
         Find out more details here: {url}\n"
    );
    Message {
        to: to.to_owned(),
        subject: format!("You have been added as a collaborator to the {resource} {class_name}"),
        text: text_layout(cfg, to, &text),
        html: html_layout(cfg, to, &html),
    }
}

/// `AdoptionMailer#interest_email`.
pub fn adoption_interest(
    cfg: &Config,
    to: &str,
    thing: &str,
    name: &str,
    url: &str,
    adopting_username: &str,
    email: &str,
) -> Message {
    let username = escape(adopting_username);
    let email = escape(email);
    let html = format!(
        "Your {thing}, {name}, which is currently up for adoption, got some interest today.<br />\n\
         <a href=\"{url}\">{name}</a>\n<br />\n<br />\n\
         Here is the information you need to know about the awesome Chef who interested in adopting your {thing}:<br />\n\
         Supermarket Username: {username}<br />\n\
         Email Address: <a href=\"mailto:{email}\">{email}</a><br />\n\n\
         Please get in touch with this Chef to discuss the potential adoption.\n"
    );
    let text = format!(
        "Your {thing}, {name}, which is currently up for adoption, got some interest today.\n\n\
         {url}\n\n\
         Here is the information you need to know about the awesome Chef who interested in adopting your {thing}:\n\
         Supermarket Username: {username}\nEmail Address: {email}\n\n\
         Please get in touch with this Chef to discuss the potential adoption.\n"
    );
    Message {
        to: to.to_owned(),
        subject: format!("Interest in adopting your {name} {thing}"),
        text: text_layout(cfg, to, &text),
        html: html_layout(cfg, to, &html),
    }
}

/// `AdoptionMailer#follower_email`.
pub fn adoption_follower(cfg: &Config, to: &str, thing: &str, name: &str, url: &str) -> Message {
    let html = format!(
        "Your {thing}, {name}, which is currently up for adoption, got some interest today.<br />\n\
         <a href=\"{url}\">{name}</a>\n<br />\n<br />\n\
         If you are interested in adopting {name}, please either contact the Supermarket maintainers or visit the {thing} page \
         in order to learn more. You can select the \"Adopt Me\" button in the sidebar from the show page for the {thing}\n"
    );
    let text = html.clone();
    Message {
        to: to.to_owned(),
        subject: format!("{name} {thing} up for adoption"),
        text: text_layout(cfg, "", &text),
        html: html_layout(cfg, "", &html),
    }
}

/// `HTML_Truncator.truncate(html, 200, ellipsis: '')` for the changelog excerpt:
/// cut at a tag boundary once the visible text exceeds the limit.
fn truncate_html(html: &str, limit: usize) -> String {
    let mut visible = 0usize;
    let mut out = String::new();
    let mut rest = html;
    while let Some(index) = rest.find('<') {
        let text = &rest[..index];
        out.push_str(text);
        visible += text.chars().count();
        rest = &rest[index..];
        let Some(end) = rest.find('>') else {
            out.push_str(rest);
            return out;
        };
        out.push_str(&rest[..=end]);
        rest = &rest[end + 1..];
        if visible >= limit {
            return out;
        }
    }
    out.push_str(rest);
    out
}

// ------------------------------------------------------------------ delivery

/// Sends via SMTP when configured, else hands the message to `sendmail`.
/// With `MAIL_CAPTURE_DIR` set, bodies are written there instead (harness).
pub async fn deliver(_cfg: &Config, message: &Message) {
    if let Some(directory) = std::env::var("MAIL_CAPTURE_DIR")
        .ok()
        .filter(|d| !d.is_empty())
    {
        capture(&directory, message);
        return;
    }
    // Delivered inline: the workers call `deliver_now`, and a process that
    // drains the queue must not exit before the message is handed off.
    let from = from_address();
    let Ok(built) = build(
        &from,
        &message.to,
        &message.subject,
        &message.text,
        &message.html,
    ) else {
        return;
    };
    match SmtpSettings::from_env() {
        Some(smtp) => smtp.send(built).await,
        None => sendmail(built).await,
    }
}

/// Writes the message as JSON, numbered so ordering stays comparable.
fn capture(directory: &str, message: &Message) {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static NEXT: AtomicUsize = AtomicUsize::new(1);
    let index = NEXT.fetch_add(1, Ordering::Relaxed);
    let path = std::path::Path::new(directory).join(format!("{index:03}.json"));
    let _ = std::fs::create_dir_all(directory);
    let payload = serde_json::json!({
        "to": message.to,
        "subject": message.subject,
        "text": message.text,
        "html": message.html,
    });
    let _ = std::fs::write(path, payload.to_string());
}

fn build(
    from: &str,
    to: &str,
    subject: &str,
    text: &str,
    html: &str,
) -> Result<lettre::Message, String> {
    use lettre::message::{header, MultiPart, SinglePart};
    let message = lettre::Message::builder()
        .from(from.parse().map_err(|error| format!("{error}"))?)
        .to(to.parse().map_err(|error| format!("{error}"))?)
        .subject(subject)
        .multipart(
            MultiPart::alternative()
                .singlepart(
                    SinglePart::builder()
                        .header(header::ContentType::TEXT_PLAIN)
                        .body(text.to_owned()),
                )
                .singlepart(
                    SinglePart::builder()
                        .header(header::ContentType::TEXT_HTML)
                        .body(html.to_owned()),
                ),
        )
        .map_err(|error| format!("{error}"))?;
    Ok(message)
}

struct SmtpSettings {
    address: String,
    port: u16,
    user: Option<String>,
    password: Option<String>,
}

impl SmtpSettings {
    fn from_env() -> Option<Self> {
        let address = std::env::var("SMTP_ADDRESS")
            .ok()
            .filter(|v| !v.is_empty())?;
        let port = std::env::var("SMTP_PORT")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(587);
        Some(Self {
            address,
            port,
            user: std::env::var("SMTP_USER_NAME")
                .ok()
                .filter(|v| !v.is_empty()),
            password: std::env::var("SMTP_PASSWORD")
                .ok()
                .filter(|v| !v.is_empty()),
        })
    }

    async fn send(self, message: lettre::Message) {
        use lettre::transport::smtp::authentication::Credentials;
        let Ok(mut builder) = lettre::SmtpTransport::relay(&self.address) else {
            return;
        };
        builder = builder.port(self.port);
        if let (Some(user), Some(password)) = (self.user, self.password) {
            builder = builder.credentials(Credentials::new(user, password));
        }
        use lettre::Transport;
        let transport = builder.build();
        let _ = transport.send(&message);
    }
}

async fn sendmail(message: lettre::Message) {
    use tokio::io::AsyncWriteExt;
    let encoded = message.formatted();
    let Ok(mut child) = tokio::process::Command::new("/usr/sbin/sendmail")
        .arg("-i")
        .arg("-t")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
    else {
        return;
    };
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(&encoded).await;
    }
    let _ = child.wait().await;
}

/// `root_url` for the `Mailer` bodies.
pub fn base(cfg: &Config) -> String {
    root_url(cfg)
}
