use lettre::message::{header::ContentType, Mailbox, Message};
use lettre::transport::smtp::authentication::Credentials;
use lettre::{AsyncSmtpTransport, AsyncTransport, Tokio1Executor};

use crate::config::EmailConfig;
use crate::error::{EmailError, Result};
use crate::validation::{validate_email, validate_not_empty, validate_max_length};

const SUBJECT_LENGTH_MAX: usize = 998;
const BODY_LENGTH_MAX: usize = 10 * 1024 * 1024;

pub struct EmailProvider {
    transport:    AsyncSmtpTransport<Tokio1Executor>,
    from_address: Mailbox,
    smtp_host:    String,
    smtp_port:    u16,
}

#[derive(Debug, Clone)]
pub struct EmailMessage {
    pub to:       String,
    pub subject:  String,
    pub body:     String,
    pub html:     bool,
}

#[derive(Debug, Clone)]
pub enum NotificationType {
    DeploymentStarted,
    DeploymentSucceeded,
    DeploymentFailed,
    SecurityAlert,
    SystemNotification,
}

impl EmailProvider {
    pub async fn new(config: &EmailConfig) -> Result<Self> {
        let credentials = Credentials::new(
            config.smtp_user.clone(),
            config.smtp_password.clone(),
        );

        let transport = AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&config.smtp_host)
            .map_err(|e| EmailError::ConnectionFailed {
                host:   config.smtp_host.clone(),
                port:   config.smtp_port,
                reason: e.to_string(),
            })?
            .port(config.smtp_port)
            .credentials(credentials)
            .build();

        let from_address: Mailbox = config.from_address
            .parse()
            .map_err(|_| EmailError::InvalidAddress {
                address: config.from_address.clone(),
            })?;

        Ok(Self {
            transport,
            from_address,
            smtp_host: config.smtp_host.clone(),
            smtp_port: config.smtp_port,
        })
    }

    pub async fn send(&self, message: EmailMessage) -> Result<()> {
        validate_not_empty(&message.to, "recipient")?;
        validate_email(&message.to)?;
        validate_not_empty(&message.subject, "subject")?;
        validate_max_length(&message.subject, SUBJECT_LENGTH_MAX, "subject")?;

        if message.body.len() > BODY_LENGTH_MAX {
            return Err(EmailError::MessageTooLarge {
                size_bytes:  message.body.len(),
                limit_bytes: BODY_LENGTH_MAX,
            }.into());
        }

        let to_mailbox: Mailbox = message.to
            .parse()
            .map_err(|_| EmailError::InvalidAddress {
                address: message.to.clone(),
            })?;

        let content_type = if message.html {
            ContentType::TEXT_HTML
        } else {
            ContentType::TEXT_PLAIN
        };

        let email = Message::builder()
            .from(self.from_address.clone())
            .to(to_mailbox.clone())
            .subject(message.subject)
            .header(content_type)
            .body(message.body)
            .map_err(|e| EmailError::SendFailed {
                to:     message.to.clone(),
                reason: e.to_string(),
            })?;

        self.transport
            .send(email)
            .await
            .map_err(|e| EmailError::SendFailed {
                to:     to_mailbox.to_string(),
                reason: e.to_string(),
            })?;

        Ok(())
    }

    pub async fn send_notification(
        &self,
        to: &str,
        notification_type: NotificationType,
        title: &str,
        details: &str,
    ) -> Result<()> {
        let (subject_prefix, color) = match notification_type {
            NotificationType::DeploymentStarted   => ("Deployment Started", "#3498db"),
            NotificationType::DeploymentSucceeded => ("Deployment Succeeded", "#27ae60"),
            NotificationType::DeploymentFailed    => ("Deployment Failed", "#e74c3c"),
            NotificationType::SecurityAlert       => ("Security Alert", "#e67e22"),
            NotificationType::SystemNotification  => ("System Notification", "#9b59b6"),
        };

        let subject = format!("{subject_prefix}: {title}");

        let body = format!(
            r#"<!DOCTYPE html>
<html>
<head>
    <meta charset="utf-8">
</head>
<body style="font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif; padding: 20px;">
    <div style="max-width: 600px; margin: 0 auto; border: 1px solid #e0e0e0; border-radius: 8px; overflow: hidden;">
        <div style="background-color: {color}; color: white; padding: 20px;">
            <h1 style="margin: 0; font-size: 24px;">{subject_prefix}</h1>
        </div>
        <div style="padding: 20px;">
            <h2 style="margin-top: 0;">{title}</h2>
            <div style="background-color: #f5f5f5; padding: 15px; border-radius: 4px; white-space: pre-wrap;">{details}</div>
        </div>
        <div style="background-color: #f9f9f9; padding: 15px; text-align: center; font-size: 12px; color: #666;">
            Sent by RustDeploy
        </div>
    </div>
</body>
</html>"#
        );

        self.send(EmailMessage {
            to:      to.to_string(),
            subject,
            body,
            html:    true,
        }).await
    }

    pub async fn send_deployment_started(
        &self,
        to: &str,
        project: &str,
        branch: &str,
        commit: &str,
    ) -> Result<()> {
        let title = format!("{project} / {branch}");
        let details = format!("Commit: {commit}\nStarting deployment...");

        self.send_notification(
            to,
            NotificationType::DeploymentStarted,
            &title,
            &details,
        ).await
    }

    pub async fn send_deployment_succeeded(
        &self,
        to: &str,
        project: &str,
        branch: &str,
        url: &str,
        duration_seconds: u64,
    ) -> Result<()> {
        let title = format!("{project} / {branch}");
        let details = format!(
            "URL: {url}\nDuration: {duration_seconds}s\n\nDeployment completed successfully."
        );

        self.send_notification(
            to,
            NotificationType::DeploymentSucceeded,
            &title,
            &details,
        ).await
    }

    pub async fn send_deployment_failed(
        &self,
        to: &str,
        project: &str,
        branch: &str,
        error: &str,
    ) -> Result<()> {
        let title = format!("{project} / {branch}");
        let details = format!("Error:\n{error}");

        self.send_notification(
            to,
            NotificationType::DeploymentFailed,
            &title,
            &details,
        ).await
    }
}
