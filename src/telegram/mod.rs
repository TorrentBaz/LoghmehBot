use std::error::Error;

use teloxide::{
    dispatching::UpdateHandler,
    dptree,
    prelude::*,
    types::{CallbackQuery, ChatId, InlineKeyboardButton, InlineKeyboardMarkup, Message, Update},
};
use tracing::{error, warn};
use uuid::Uuid;

use crate::{
    db::{CreatedOrder, ServiceOffering},
    state::AppState,
};

type HandlerResult = Result<(), Box<dyn Error + Send + Sync + 'static>>;

pub fn schema() -> UpdateHandler<Box<dyn Error + Send + Sync + 'static>> {
    dptree::entry()
        .branch(Update::filter_message().endpoint(handle_message))
        .branch(Update::filter_callback_query().endpoint(handle_callback))
}

async fn handle_message(bot: Bot, msg: Message, state: AppState) -> HandlerResult {
    let Some(from) = msg.from.as_ref() else {
        return Ok(());
    };
    let telegram_id = from.id.0 as i64;
    let user = match state
        .db
        .upsert_telegram_user(
            telegram_id,
            from.username.as_deref(),
            &from.first_name,
            state.config.is_telegram_admin(telegram_id),
        )
        .await
    {
        Ok(user) => user,
        Err(error) => {
            error!(%error, "could not persist Telegram user");
            bot.send_message(
                msg.chat.id,
                "مشکل موقتی در اتصال فروشگاه پیش آمد. لطفاً دوباره تلاش کن.",
            )
            .await?;
            return Ok(());
        }
    };
    let text = msg.text().unwrap_or("").trim();

    if text.starts_with("/start") {
        bot.send_message(
            msg.chat.id,
            "سلام، به Loghmeh خوش اومدی 👋\nبرای دیدن سرویس‌ها روی «مشاهده سرویس‌ها» بزن.",
        )
        .reply_markup(main_menu())
        .await?;
        return Ok(());
    }
    if text == "/services" || text == "مشاهده سرویس‌ها" {
        show_services(&bot, msg.chat.id, &state).await?;
        return Ok(());
    }
    if text == "/orders" || text == "خریدهای من" {
        show_user_orders(&bot, msg.chat.id, user.id, &state).await?;
        return Ok(());
    }
    if text == "/admin" {
        if !state.config.is_telegram_admin(telegram_id) {
            bot.send_message(msg.chat.id, "این بخش فقط برای مدیر فروشگاه است.")
                .await?;
            return Ok(());
        }
        show_admin_summary(&bot, msg.chat.id, &state).await?;
        return Ok(());
    }

    if let Some(offering_id) = state.take_selected_offering(telegram_id).await {
        let Some(target_username) = parse_target_username(text) else {
            state.select_offering(telegram_id, offering_id).await;
            bot.send_message(
                msg.chat.id,
                "نام کاربری مقصد معتبر نیست. فقط نام کاربری تلگرام را با @ بفرست؛ مثل @sample_user",
            )
            .await?;
            return Ok(());
        };
        match state
            .db
            .create_checkout(user.id, offering_id, &target_username)
            .await
        {
            Ok(checkout_id) => {
                bot.send_message(
                    msg.chat.id,
                    format!(
                        "مقصد: {target_username}\nاگر درست است، پرداخت را ادامه بده. این مرحله ۱۵ دقیقه اعتبار دارد."
                    ),
                )
                .reply_markup(InlineKeyboardMarkup::new(vec![vec![
                    InlineKeyboardButton::callback("ادامه و پرداخت", format!("confirm:{checkout_id}")),
                    InlineKeyboardButton::callback("لغو", format!("cancel:{checkout_id}")),
                ]]))
                .await?;
            }
            Err(error) => {
                warn!(%error, "could not create checkout");
                bot.send_message(
                    msg.chat.id,
                    "این سرویس فعلاً در دسترس نیست. لطفاً از فهرست سرویس‌ها دوباره انتخاب کن.",
                )
                .await?;
            }
        }
        return Ok(());
    }

    bot.send_message(
        msg.chat.id,
        "برای شروع، «مشاهده سرویس‌ها» یا /services را بزن.",
    )
    .reply_markup(main_menu())
    .await?;
    Ok(())
}

async fn handle_callback(bot: Bot, query: CallbackQuery, state: AppState) -> HandlerResult {
    let telegram_id = query.from.id.0 as i64;
    let chat_id = ChatId(telegram_id);
    let user = match state
        .db
        .upsert_telegram_user(
            telegram_id,
            query.from.username.as_deref(),
            &query.from.first_name,
            state.config.is_telegram_admin(telegram_id),
        )
        .await
    {
        Ok(user) => user,
        Err(error) => {
            error!(%error, "could not persist callback user");
            bot.answer_callback_query(query.id)
                .text("خطای موقت؛ دوباره تلاش کن.")
                .await?;
            return Ok(());
        }
    };
    let data = query.data.unwrap_or_default();
    bot.answer_callback_query(query.id).await?;

    if data == "services" {
        show_services(&bot, chat_id, &state).await?;
        return Ok(());
    }
    if data == "orders" {
        show_user_orders(&bot, chat_id, user.id, &state).await?;
        return Ok(());
    }
    if data == "admin:orders" && state.config.is_telegram_admin(telegram_id) {
        show_admin_orders(&bot, chat_id, &state).await?;
        return Ok(());
    }
    if let Some(value) = data.strip_prefix("offer:") {
        match Uuid::parse_str(value) {
            Ok(offering_id) => {
                state.select_offering(telegram_id, offering_id).await;
                bot.send_message(
                    chat_id,
                    "نام کاربری تلگرامِ دریافت‌کننده را با @ بفرست.\nمثال: @sample_user",
                )
                .await?;
            }
            Err(_) => {
                bot.send_message(
                    chat_id,
                    "انتخاب سرویس نامعتبر است. لطفاً دوباره از فهرست انتخاب کن.",
                )
                .await?;
            }
        }
        return Ok(());
    }
    if let Some(value) = data.strip_prefix("cancel:") {
        if let Ok(checkout_id) = Uuid::parse_str(value)
            && let Err(error) = state.db.cancel_checkout(checkout_id, user.id).await
        {
            error!(%error, "could not cancel checkout");
        }
        bot.send_message(chat_id, "خرید لغو شد.")
            .reply_markup(main_menu())
            .await?;
        return Ok(());
    }
    if let Some(value) = data.strip_prefix("confirm:") {
        let Ok(checkout_id) = Uuid::parse_str(value) else {
            bot.send_message(
                chat_id,
                "درخواست پرداخت نامعتبر است. لطفاً دوباره سرویس را انتخاب کن.",
            )
            .await?;
            return Ok(());
        };
        let order = match state.db.finalize_checkout(checkout_id, user.id).await {
            Ok(order) => order,
            Err(error) => {
                warn!(%error, "could not finalize checkout");
                bot.send_message(
                    chat_id,
                    "زمان این پرداخت تمام شده یا قبلاً استفاده شده است. لطفاً دوباره سفارش بساز.",
                )
                .await?;
                return Ok(());
            }
        };
        create_payment_and_reply(&bot, chat_id, &state, order).await?;
        return Ok(());
    }
    Ok(())
}

async fn create_payment_and_reply(
    bot: &Bot,
    chat_id: ChatId,
    state: &AppState,
    order: CreatedOrder,
) -> HandlerResult {
    let callback_url = format!("{}/webhooks/aban", state.config.public_base_url);
    let description = format!("Loghmeh | {} | {}", order.title, order.public_id);
    match state
        .aban
        .create_invoice(
            order.amount_rial,
            &order.public_id,
            &callback_url,
            &description,
        )
        .await
    {
        Ok(invoice) => {
            if let Err(error) = state.db.register_aban_invoice(&order, &invoice).await {
                error!(%error, order = %order.public_id, "could not record invoice");
                bot.send_message(
                    chat_id,
                    "فاکتور ساخته شد ولی ثبت آن مشکل دارد؛ فعلاً پرداخت نکن و به پشتیبانی پیام بده.",
                )
                .await?;
                return Ok(());
            }
            let summary = format!(
                "سفارش {}\n{} برای {}\nمبلغ: {} ریال\n\nبعد از پرداخت، وضعیت سفارش خودکار به‌روزرسانی می‌شود.",
                order.public_id,
                order.title,
                order.target_username,
                format_rial(invoice.payable_rial),
            );
            match invoice.payment_url.parse::<reqwest::Url>() {
                Ok(url) => {
                    bot.send_message(chat_id, summary)
                        .reply_markup(InlineKeyboardMarkup::new(vec![vec![
                            InlineKeyboardButton::url("پرداخت امن", url),
                        ]]))
                        .await?;
                }
                Err(error) => {
                    error!(%error, "AbanGateway returned an invalid payment URL");
                    bot.send_message(
                        chat_id,
                        "لینک پرداخت نامعتبر است؛ فعلاً پرداخت نکن و به پشتیبانی پیام بده.",
                    )
                    .await?;
                }
            }
        }
        Err(error) => {
            error!(%error, order = %order.public_id, "could not create AbanGateway invoice");
            let _ = state
                .db
                .mark_payment_creation_failed(order.id, &error.to_string())
                .await;
            bot.send_message(chat_id, "فعلاً نتوانستم فاکتور پرداخت بسازم. هیچ مبلغی پرداخت نشده؛ چند دقیقه دیگر دوباره امتحان کن.").await?;
        }
    }
    Ok(())
}

async fn show_services(bot: &Bot, chat_id: ChatId, state: &AppState) -> HandlerResult {
    match state.db.active_offerings().await {
        Ok(offerings) if offerings.is_empty() => {
            bot.send_message(
                chat_id,
                "هنوز سرویسی برای فروش فعال نشده است. کمی بعد دوباره سر بزن.",
            )
            .await?;
        }
        Ok(offerings) => {
            let text = "سرویس موردنظرت را انتخاب کن. قبل از پرداخت، نام کاربری گیرنده را می‌پرسیم.";
            bot.send_message(chat_id, text)
                .reply_markup(offerings_keyboard(&offerings))
                .await?;
        }
        Err(error) => {
            error!(%error, "could not list services");
            bot.send_message(
                chat_id,
                "فهرست سرویس‌ها فعلاً در دسترس نیست. لطفاً دوباره تلاش کن.",
            )
            .await?;
        }
    }
    Ok(())
}

async fn show_user_orders(
    bot: &Bot,
    chat_id: ChatId,
    user_id: Uuid,
    state: &AppState,
) -> HandlerResult {
    match state.db.list_user_orders(user_id).await {
        Ok(orders) if orders.is_empty() => {
            bot.send_message(chat_id, "هنوز سفارشی نداری.").await?;
        }
        Ok(orders) => {
            bot.send_message(chat_id, format_orders(&orders)).await?;
        }
        Err(error) => {
            error!(%error, "could not list customer orders");
            bot.send_message(chat_id, "فعلاً نتوانستم سفارش‌ها را بخوانم.")
                .await?;
        }
    };
    Ok(())
}

async fn show_admin_summary(bot: &Bot, chat_id: ChatId, state: &AppState) -> HandlerResult {
    match state.db.dashboard_summary().await {
        Ok(summary) => {
            bot.send_message(
                chat_id,
                format!(
                    "پنل مدیریت Loghmeh\n\nسرویس فعال: {}\nدر انتظار پرداخت: {}\nنیازمند بررسی: {}\nفروش امروز: {} ریال",
                    summary.active_offerings,
                    summary.orders_awaiting_payment,
                    summary.jobs_needing_attention,
                    format_rial(summary.paid_today_rial),
                ),
            )
            .reply_markup(InlineKeyboardMarkup::new(vec![vec![InlineKeyboardButton::callback("سفارش‌های اخیر", "admin:orders")]]))
            .await?;
        }
        Err(error) => {
            error!(%error, "could not get dashboard summary");
            bot.send_message(chat_id, "پنل مدیریت فعلاً در دسترس نیست.")
                .await?;
        }
    }
    Ok(())
}

async fn show_admin_orders(bot: &Bot, chat_id: ChatId, state: &AppState) -> HandlerResult {
    match state.db.list_recent_orders(10).await {
        Ok(orders) if orders.is_empty() => {
            bot.send_message(chat_id, "هنوز سفارشی ثبت نشده است.")
                .await?;
        }
        Ok(orders) => {
            bot.send_message(chat_id, format_orders(&orders)).await?;
        }
        Err(error) => {
            error!(%error, "could not get admin orders");
            bot.send_message(chat_id, "فعلاً نتوانستم سفارش‌ها را بخوانم.")
                .await?;
        }
    };
    Ok(())
}

fn main_menu() -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![
        vec![InlineKeyboardButton::callback("مشاهده سرویس‌ها", "services")],
        vec![InlineKeyboardButton::callback("خریدهای من", "orders")],
    ])
}

fn offerings_keyboard(offerings: &[ServiceOffering]) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(
        offerings
            .iter()
            .map(|offering| {
                vec![InlineKeyboardButton::callback(
                    format!(
                        "{} — {} ریال",
                        offering.title,
                        format_rial(offering.price_rial)
                    ),
                    format!("offer:{}", offering.id),
                )]
            })
            .collect::<Vec<_>>(),
    )
}

fn format_orders(orders: &[crate::db::AdminOrderView]) -> String {
    orders
        .iter()
        .map(|order| {
            format!(
                "{}\n{} → {}\n{} ریال | {}",
                order.public_id,
                order.service_title,
                order.target_username,
                format_rial(order.amount_rial),
                order.status
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn format_rial(amount: i64) -> String {
    let digits = amount.abs().to_string();
    let mut output = String::new();
    for (index, character) in digits.chars().rev().enumerate() {
        if index > 0 && index % 3 == 0 {
            output.push(',');
        }
        output.push(character);
    }
    if amount < 0 {
        output.push('-');
    }
    output.chars().rev().collect()
}

fn parse_target_username(value: &str) -> Option<String> {
    let username = value.trim().strip_prefix('@')?;
    if !(5..=32).contains(&username.len())
        || !username
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    {
        return None;
    }
    Some(format!("@{username}"))
}

#[cfg(test)]
mod tests {
    use super::{format_rial, parse_target_username};

    #[test]
    fn validates_telegram_usernames() {
        assert_eq!(
            parse_target_username("@loghmeh_bot"),
            Some("@loghmeh_bot".into())
        );
        assert!(parse_target_username("loghmeh_bot").is_none());
        assert!(parse_target_username("@bad-name").is_none());
    }

    #[test]
    fn formats_rial() {
        assert_eq!(format_rial(1250000), "1,250,000");
    }
}
