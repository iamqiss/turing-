-- Turing Tarpit Cloud: nodes, workloads, instances, IP pools
-- CloudRat lessons: physical_cores truth, overcommit, NAT port ranges,
-- reserved quarantine cores, IPv6-default.

CREATE TABLE IF NOT EXISTS compute_nodes (
    id                  UUID PRIMARY KEY,
    name                TEXT NOT NULL,
    region              TEXT NOT NULL,
    status              TEXT NOT NULL DEFAULT 'registering',
    physical_cores      INT  NOT NULL,
    memory_mb           INT  NOT NULL,
    reserved_cores      INT  NOT NULL DEFAULT 0,
    reserved_memory_mb  INT  NOT NULL DEFAULT 0,
    overcommit_ratio    REAL NOT NULL DEFAULT 4.0,
    agent_endpoint      TEXT,
    last_heartbeat      TIMESTAMPTZ,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS compute_nodes_region_idx ON compute_nodes (region, status);

CREATE TABLE IF NOT EXISTS image_templates (
    id              UUID PRIMARY KEY,
    name            TEXT NOT NULL,
    slug            TEXT NOT NULL UNIQUE,
    description     TEXT,
    disk_mb         INT  NOT NULL DEFAULT 800,
    active          BOOLEAN NOT NULL DEFAULT TRUE
);

CREATE TABLE IF NOT EXISTS workloads (
    id                  UUID PRIMARY KEY,
    org_id              UUID NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    name                TEXT NOT NULL,
    shape               TEXT NOT NULL,
    status              TEXT NOT NULL DEFAULT 'pending',
    vcpu                INT  NOT NULL DEFAULT 1,
    memory_mb           INT  NOT NULL DEFAULT 512,
    disk_gb             INT  NOT NULL DEFAULT 10,
    image_template_id   UUID REFERENCES image_templates(id),
    repository_id       UUID REFERENCES repositories(id) ON DELETE SET NULL,
    git_ref             TEXT,
    git_sha             TEXT,
    region              TEXT NOT NULL,
    created_by          UUID NOT NULL REFERENCES principals(id),
    created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS workloads_org_idx ON workloads (org_id, status);
CREATE INDEX IF NOT EXISTS workloads_region_idx ON workloads (region, status);

CREATE TRIGGER workloads_updated_at
    BEFORE UPDATE ON workloads
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

CREATE TABLE IF NOT EXISTS instances (
    id                  UUID PRIMARY KEY,
    workload_id         UUID NOT NULL REFERENCES workloads(id) ON DELETE CASCADE,
    node_id             UUID NOT NULL REFERENCES compute_nodes(id),
    driver              TEXT NOT NULL,
    status              TEXT NOT NULL DEFAULT 'pending',
    private_ip          TEXT,
    ipv6                TEXT,
    nat_public_ip       TEXT,
    nat_port_start      INT,
    nat_port_end        INT,
    dedicated_ipv4      TEXT,
    hypervisor_id       TEXT,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS instances_workload_idx ON instances (workload_id);
CREATE INDEX IF NOT EXISTS instances_node_idx ON instances (node_id, status);

CREATE TRIGGER instances_updated_at
    BEFORE UPDATE ON instances
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

CREATE TABLE IF NOT EXISTS subnets (
    id          UUID PRIMARY KEY,
    region      TEXT NOT NULL,
    cidr        TEXT NOT NULL,
    kind        TEXT NOT NULL,
    node_id     UUID REFERENCES compute_nodes(id) ON DELETE SET NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS ip_allocations (
    id              UUID PRIMARY KEY,
    subnet_id       UUID NOT NULL REFERENCES subnets(id) ON DELETE CASCADE,
    instance_id     UUID REFERENCES instances(id) ON DELETE SET NULL,
    address         TEXT NOT NULL,
    port_start      INT,
    port_end        INT,
    kind            TEXT NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (subnet_id, address, port_start)
);

CREATE INDEX IF NOT EXISTS ip_allocations_instance_idx ON ip_allocations (instance_id);

-- Org compute quotas (protect margin)
CREATE TABLE IF NOT EXISTS org_compute_quotas (
    org_id                  UUID PRIMARY KEY REFERENCES orgs(id) ON DELETE CASCADE,
    max_workloads           INT NOT NULL DEFAULT 5,
    max_vcpu_total          INT NOT NULL DEFAULT 8,
    max_concurrent_running  INT NOT NULL DEFAULT 5,
    updated_at              TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Seed curated templates (CloudRat-style small footprints)
INSERT INTO image_templates (id, name, slug, description, disk_mb, active)
VALUES
    ('01900000-0000-7000-8000-000000000001', 'Ubuntu 24.04 LTS Minimal', 'ubuntu-24.04', 'General-purpose default', 800, TRUE),
    ('01900000-0000-7000-8000-000000000002', 'Docker Host', 'docker-host', 'Ubuntu + Docker/Compose', 1200, TRUE),
    ('01900000-0000-7000-8000-000000000003', 'Alpine Linux', 'alpine', 'Tiny sandboxes, max density', 50, TRUE)
ON CONFLICT DO NOTHING;
