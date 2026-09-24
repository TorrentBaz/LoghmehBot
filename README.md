# Loghmeh

Loghmeh یک ربات فروش ماژولار برای خدمات دیجیتال است. هسته با **Rust** نوشته شده، اطلاعات را در **PostgreSQL** نگه می‌دارد و پرداخت تومانی را با **AbanGateway** انجام می‌دهد.

در این نسخه داریم:

- خرید ساده در Telegram: انتخاب سرویس → واردکردن نام کاربری مقصد → پرداخت → پیگیری سفارش.
- پنل تحت‌وب فارسی و سبک برای ساخت/فعال‌کردن سرویس و دیدن سفارش‌ها.
- وب‌هوک امضاشده و تأیید یک‌بارمصرف برای آبان‌گیت‌وی؛ پرداخت جعلی یا تکراری تحویل نمی‌گیرد.
- صف تحویل مقاوم: هر ابهام به بررسی دستی می‌رود، نه پرداخت یا تحویل دوباره.
- اتصال آماده به Connector جدا برای خودکارسازی Fragment؛ قرارداد آن در [docs/fragment-connector-contract.md](docs/fragment-connector-contract.md) است.
- ساختار افزونه‌ای برای افزودن پرداخت کریپتو، خدمات جدید، موجودی، تخفیف و گزارش‌های کامل.

## نقشهٔ خیلی ساده

```text
مشتری در Telegram
        ↓
Loghmeh (Rust) ←→ PostgreSQL
        ↓                  ↓
  AbanGateway        صف تحویل
                           ↓
                    Fragment Connector جداگانه
```

## کاری که تو باید انجام بدهی

این مراحل را به‌ترتیب انجام بده؛ هنوز هیچ پول واقعی جابه‌جا نمی‌شود تا وقتی خودت سرویس را فعال کنی.

1. در Telegram به `@BotFather` پیام بده، `/newbot` را بزن، نام را `Loghmeh` قرار بده و برای نام کاربری `LoghmehBot` را امتحان کن. اگر آزاد نبود، Telegram خودش از تو نام دیگری می‌خواهد؛ آن وقت یک پسوند کوتاه مثل `LoghmehShopBot` انتخاب کن. توکنِ داده‌شده را جایی خصوصی نگه دار.
2. شناسهٔ عددی خودت را از یک ربات نمایش‌دهندهٔ ID بگیر. این شناسه فقط برای بازشدن دستور `/admin` به خودت است.
3. در AbanGateway حساب بساز، ابتدا توکن `test_` و Webhook Secret محیط آزمایشی را بردار. طبق مستندات آبان، مبلغ API همیشه **ریال** است، نه تومان.
4. در ویندوز یک‌بار [Visual Studio Build Tools برای C++](https://visualstudio.microsoft.com/thank-you-for-downloading-visual-studio-for-cplusplus/) را نصب کن و در نصب‌کننده گزینهٔ «Desktop development with C++» را تیک بزن. بدون آن Rust روی این ویندوز برنامه را build نمی‌کند.
5. اگر Docker Desktop داری، در پوشهٔ پروژه این دستور را اجرا کن تا PostgreSQL روشن شود:

   ```powershell
   docker compose up -d
   ```

6. فایل نمونهٔ تنظیمات را کپی کن و مقادیر مرحله‌های قبل را در آن بچسبان:

   ```powershell
   Copy-Item .env.example .env
   ```

   فایل `.env` شخصی است؛ هرگز آن را به GitHub نفرست.
7. برنامه را اجرا کن:

   ```powershell
   cargo run
   ```

8. مرورگر را روی `http://localhost:8080/admin` باز کن، مقدار `ADMIN_API_TOKEN` را وارد کن و سرویس‌ها را بساز. تا وقتی تیک «فعال» را نزده‌ای، مشتری آن سرویس را نمی‌بیند.
9. با توکن آزمایشی آبان، چرخهٔ کامل ساخت فاکتور، وب‌هوک و verify را تست کن. فقط بعد از تست موفق، توکن `live_` را در `.env` قرار بده.

برای پرداخت واقعی، `PUBLIC_BASE_URL` باید یک آدرس HTTPS عمومی باشد؛ `localhost` فقط برای تست محلی است.

## اتصال خودکار Premium و Stars

برای روشن‌کردن مسیر خودکار Fragment، ابتدا Connector جدا را آماده و آزمایش کن؛ جزئیات دقیق ورودی/خروجی در [docs/fragment-connector-contract.md](docs/fragment-connector-contract.md) آمده است. سپس در `.env` این سه مقدار را پر کن:

```dotenv
FULFILLMENT_PROVIDER=fragment_connector
FULFILLMENT_ENDPOINT=https://connector.example.com/fulfill
FULFILLMENT_HMAC_SECRET=یک_رمز_بلند_و_تصادفی
```

تا قبل از آن `FULFILLMENT_PROVIDER=manual` بماند؛ سفارش پس از پرداخت در بخش بررسی مدیر قرار می‌گیرد.

برای Premium یک مسیر رسمیِ دوم هم وجود دارد: Telegram Bot API متد `giftPremiumSubscription` را دارد، اما هزینه‌اش از موجودی Stars خود ربات پرداخت می‌شود و به Telegram user ID گیرنده نیاز دارد؛ پس جایگزین کامل فروش Stars به نام کاربری دلخواه نیست. این مسیر را به‌عنوان ارائه‌دهندهٔ بعدی نگه می‌داریم؛ مسیر منتخب نسخهٔ اول همان Fragment Connector است و قرارداد اتصالش آماده است.

## پرداخت کریپتو

هستهٔ سفارش و جدول پرداخت برای `crypto` آماده است، اما ارائه‌دهندهٔ کریپتو عمداً فعال نشده است. Cryptomus در مدارک AML/KYC خود اعلام می‌کند که ممکن است اطلاعات هویتی لازم باشد؛ بنابراین آن را «بی‌نیاز از احراز هویت» فرض نکن. قبل از اتصال هر درگاه، شرایط سرویس، کشورها/تحریم‌های پشتیبانی‌شده و الزامات قانونی کسب‌وکارت را با خود آن سرویس بررسی کن. افزودن آن فقط به معنی ساخت یک `PaymentProvider` جدید است و به سایر سفارش‌ها آسیبی نمی‌زند.

## انتشار در GitHub

پس از اینکه اولین تست محلی موفق بود، در GitHub یک repository خالی به نام `loghmeh` بساز. سپس این چهار خط را در همین پوشه اجرا کن و به‌جای `YOUR_GITHUB_USERNAME` نام کاربری GitHub خودت را بگذار:

```powershell
git add .
git commit -m "Initial Loghmeh storefront"
git remote add origin https://github.com/YOUR_GITHUB_USERNAME/loghmeh.git
git push -u origin main
```

فایل `.env` به‌خاطر `.gitignore` به مخزن نمی‌رود. GitHub Actions در [.github/workflows/ci.yml](.github/workflows/ci.yml) فرمت، lint و تست را برای هر push بررسی می‌کند.

## توسعهٔ بعدی

- کد تخفیف و referral
- قیمت‌گذاری گروهی/همکاران
- موجودی و تحویل فایل یا کد
- پنل نقش‌محور برای اپراتورها
- اعلان موجودی TON و گزارش مالی
- ارائه‌دهندهٔ کریپتوی تأییدشده

جزئیات فنی در [docs/architecture.md](docs/architecture.md) و تصمیم‌های امنیتی در [docs/security-decision.md](docs/security-decision.md) هستند.
