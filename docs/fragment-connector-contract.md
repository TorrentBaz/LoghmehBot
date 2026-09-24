# قرارداد Connector خودکار Fragment

این Connector همان جریان مطرح‌شده در راهنمای ارسالی را از برنامهٔ اصلی جدا می‌کند: بررسی نام کاربری، ساخت خرید Premium یا Stars در Fragment و امضای پرداخت با کیف‌پول فروش.

## مرز مسئولیت

`loghmeh` سفارش و پرداخت تومان را مدیریت می‌کند. Connector تنها پس از تأیید پرداخت، درخواست امضاشدهٔ تحویل را می‌گیرد و به وضعیت سفارش جواب می‌دهد.

```text
Loghmeh ── HMAC POST ──> Fragment Connector ──> Fragment / Tonkeeper sales wallet
```

Connector باید روی سرویس یا ماشین جدا، با حساب Telegram و کیف‌پول مخصوص فروش اجرا شود. عبارت بازیابی و کوکی نشست در Telegram، PostgreSQL، گزارش‌ها یا مخزن Git قرار نمی‌گیرند. اگر Connector نیاز به این داده‌ها دارد، مدیر سرور آن‌ها را مستقیماً در secret store محیط همان Connector قرار می‌دهد.

## درخواست از Loghmeh

`POST $FULFILLMENT_ENDPOINT`

هدرها:

```text
Content-Type: application/json
X-Loghmeh-Signature: HMAC-SHA256 hex of the exact request body
```

بدنه:

```json
{
  "order_id": "UUID",
  "public_order_id": "LGM-…",
  "target_username": "@recipient",
  "offering": {
    "kind": "telegram_premium",
    "configuration": { "months": 3 },
    "cost_cap_nano_ton": 1234567890
  }
}
```

`order_id` کلید idempotency است: اجرای دوبارهٔ همان درخواست نباید خرید جدیدی بسازد. Connector باید قبل از ارسال TON، نام کاربری، نوع محصول، مدت/تعداد و سقف هزینه را دوباره بررسی کند.

## پاسخ Connector

```json
{ "status": "succeeded", "reference": "ton-transaction-or-provider-reference" }
```

وضعیت‌های مجاز:

- `succeeded`: تحویل قطعی انجام شده؛ `reference` الزامی است.
- `retryable_failure`: هنوز هیچ پرداختی از کیف‌پول خارج نشده و تلاش مجدد بی‌خطر است.
- `manual_review`: وضعیت انتقال یا نشست مبهم است؛ Loghmeh تلاش خودکار دیگری نمی‌کند.
- `permanent_failure`: تحویل ممکن نیست و سفارش برای رسیدگی مدیر می‌رود.

## قبل از روشن‌کردن فروش

1. Connector را با سفارش آزمایشی و موجودی کم امتحان کنید.
2. برای هر سرویس `cost_cap_nano_ton` تعیین کنید؛ بدون سقف هزینه آن را فعال نکنید.
3. توقف خودکار فروش هنگام کمبود موجودی و ارسال هشدار به مدیر را در Connector روشن کنید.
4. فقط روی خطای «قبل از ارسال پول» پاسخ `retryable_failure` بدهید. در هر وضعیت نامعلوم باید `manual_review` برگردد.

منطق خصوصی Fragment یا Tonkeeper در خود Connector باقی می‌ماند؛ هستهٔ فروشگاه به endpointهای مرورگر یا داده‌های نشست وابسته نیست.

