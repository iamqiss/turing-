-- Managed data: Turing-only brands (powered-by upstream shown in UI)
ALTER TABLE billing_managed_resources DROP CONSTRAINT IF EXISTS billing_managed_resources_kind_check;

ALTER TABLE billing_managed_resources
    ADD CONSTRAINT billing_managed_resources_kind_check
    CHECK (kind IN (
        'postgres',
        'turing_cache',
        'turing_store',
        'turing_chronos',
        -- legacy aliases still accepted by parse(); normalize on write
        'emberdb', 'rustfs', 'greptimedb',
        'ember_vanilla', 'rustfs_vanilla', 'greptime_vanilla'
    ));
