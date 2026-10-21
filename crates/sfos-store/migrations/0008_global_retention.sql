CREATE TABLE IF NOT EXISTS global_retention (
    singleton boolean PRIMARY KEY DEFAULT true CHECK (singleton),
    retention_hours integer NOT NULL CHECK (retention_hours BETWEEN 1 AND 168),
    updated_at timestamptz NOT NULL DEFAULT now()
);
