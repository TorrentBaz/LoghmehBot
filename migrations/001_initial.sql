CREATE TABLE users (
    id UUID PRIMARY KEY,
    telegram_id BIGINT NOT NULL UNIQUE,
    username TEXT,
    first_name TEXT NOT NULL,
    role TEXT NOT NULL DEFAULT 'customer' CHECK (role IN ('customer', 'admin', 'operator')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE service_categories (
    id UUID PRIMARY KEY,
    slug TEXT NOT NULL UNIQUE,
    title TEXT NOT NULL,
    description TEXT,
    sort_order INTEGER NOT NULL DEFAULT 0,
    is_active BOOLEAN NOT NULL DEFAULT TRUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE service_offerings (
    id UUID PRIMARY KEY,
    category_id UUID REFERENCES service_categories(id) ON DELETE SET NULL,
    code TEXT NOT NULL UNIQUE,
    kind TEXT NOT NULL CHECK (kind IN ('telegram_premium', 'telegram_stars', 'manual', 'external')),
    title TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    price_rial BIGINT NOT NULL CHECK (price_rial > 0),
    configuration JSONB NOT NULL DEFAULT '{}'::jsonb,
    cost_cap_nano_ton BIGINT CHECK (cost_cap_nano_ton IS NULL OR cost_cap_nano_ton > 0),
    is_active BOOLEAN NOT NULL DEFAULT FALSE,
    sort_order INTEGER NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE checkout_sessions (
    id UUID PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users(id),
    offering_id UUID NOT NULL REFERENCES service_offerings(id),
    target_username TEXT NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX checkout_sessions_user_expires_idx ON checkout_sessions (user_id, expires_at DESC);

CREATE TABLE orders (
    id UUID PRIMARY KEY,
    public_id TEXT NOT NULL UNIQUE,
    user_id UUID NOT NULL REFERENCES users(id),
    offering_id UUID NOT NULL REFERENCES service_offerings(id),
    offering_snapshot JSONB NOT NULL,
    target_username TEXT NOT NULL,
    amount_rial BIGINT NOT NULL CHECK (amount_rial > 0),
    status TEXT NOT NULL CHECK (status IN (
        'payment_creating', 'awaiting_payment', 'paid', 'fulfillment_processing',
        'fulfilled', 'requires_review', 'cancelled', 'refunded', 'failed'
    )),
    payment_expires_at TIMESTAMPTZ,
    paid_at TIMESTAMPTZ,
    fulfilled_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX orders_user_created_idx ON orders (user_id, created_at DESC);
CREATE INDEX orders_status_created_idx ON orders (status, created_at DESC);

CREATE TABLE payments (
    id UUID PRIMARY KEY,
    order_id UUID NOT NULL REFERENCES orders(id),
    provider TEXT NOT NULL CHECK (provider IN ('aban', 'crypto')),
    external_id TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('created', 'pending', 'paid', 'expired', 'cancelled', 'failed')),
    amount_rial BIGINT NOT NULL CHECK (amount_rial > 0),
    payment_url TEXT,
    provider_payload JSONB NOT NULL DEFAULT '{}'::jsonb,
    verified_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (provider, external_id)
);

CREATE INDEX payments_order_created_idx ON payments (order_id, created_at DESC);

CREATE TABLE fulfillment_jobs (
    id UUID PRIMARY KEY,
    order_id UUID NOT NULL UNIQUE REFERENCES orders(id),
    provider TEXT NOT NULL CHECK (provider IN ('manual', 'external')),
    status TEXT NOT NULL CHECK (status IN ('queued', 'processing', 'retryable_failure', 'manual_review', 'succeeded', 'failed')),
    attempt_count INTEGER NOT NULL DEFAULT 0 CHECK (attempt_count >= 0),
    next_attempt_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    locked_at TIMESTAMPTZ,
    locked_by TEXT,
    last_error TEXT,
    provider_reference TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX fulfillment_jobs_poll_idx ON fulfillment_jobs (status, next_attempt_at)
    WHERE status IN ('queued', 'retryable_failure');

CREATE TABLE order_events (
    id UUID PRIMARY KEY,
    order_id UUID NOT NULL REFERENCES orders(id),
    actor_type TEXT NOT NULL CHECK (actor_type IN ('customer', 'admin', 'system', 'payment_provider', 'fulfillment_provider')),
    actor_id TEXT,
    event_type TEXT NOT NULL,
    payload JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX order_events_order_created_idx ON order_events (order_id, created_at DESC);

CREATE TABLE webhook_deliveries (
    id UUID PRIMARY KEY,
    provider TEXT NOT NULL,
    delivery_id TEXT NOT NULL,
    event_type TEXT NOT NULL,
    signature_verified BOOLEAN NOT NULL,
    payload JSONB NOT NULL,
    received_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    handled_at TIMESTAMPTZ,
    UNIQUE (provider, delivery_id)
);

CREATE TABLE admin_audit_logs (
    id UUID PRIMARY KEY,
    admin_user_id UUID REFERENCES users(id),
    source TEXT NOT NULL CHECK (source IN ('telegram', 'web', 'system')),
    action TEXT NOT NULL,
    subject_type TEXT,
    subject_id TEXT,
    payload JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE OR REPLACE FUNCTION set_updated_at()
RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = now();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER users_set_updated_at BEFORE UPDATE ON users
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();
CREATE TRIGGER service_categories_set_updated_at BEFORE UPDATE ON service_categories
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();
CREATE TRIGGER service_offerings_set_updated_at BEFORE UPDATE ON service_offerings
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();
CREATE TRIGGER orders_set_updated_at BEFORE UPDATE ON orders
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();
CREATE TRIGGER payments_set_updated_at BEFORE UPDATE ON payments
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();
CREATE TRIGGER fulfillment_jobs_set_updated_at BEFORE UPDATE ON fulfillment_jobs
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

