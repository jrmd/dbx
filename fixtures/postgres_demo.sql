-- DBX PostgreSQL demo fixture
--
-- This is intentionally PostgreSQL-specific and is designed for a disposable
-- database. It creates roughly 90,000 rows across several schemas, including
-- foreign keys, composite keys, views, JSONB, UUID, NUMERIC, BYTEA, dates,
-- timestamps, nullable values, quoted identifiers, and long text.
--
-- Re-running the file removes only these fixture schemas:
--   dbx_demo, dbx_analytics, and dbx_support
--
-- Load it with psql, or use DBX's SQL database import action against a test
-- database. The statements are deliberately ordered as schema first, data
-- second, with referenced tables created before dependent tables.

DROP SCHEMA IF EXISTS dbx_support CASCADE;
DROP SCHEMA IF EXISTS dbx_analytics CASCADE;
DROP SCHEMA IF EXISTS dbx_demo CASCADE;

CREATE SCHEMA dbx_demo;
CREATE SCHEMA dbx_analytics;
CREATE SCHEMA dbx_support;

-- ---------------------------------------------------------------------------
-- Schema
-- ---------------------------------------------------------------------------

CREATE TABLE dbx_demo.organizations (
    id bigint PRIMARY KEY,
    slug text NOT NULL UNIQUE,
    name text NOT NULL,
    plan text NOT NULL DEFAULT 'free'
        CHECK (plan IN ('free', 'team', 'business', 'enterprise')),
    active boolean NOT NULL DEFAULT true,
    settings jsonb NOT NULL DEFAULT '{}'::jsonb,
    monthly_budget numeric(12, 2) NOT NULL DEFAULT 0
        CHECK (monthly_budget >= 0),
    created_at timestamptz NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE dbx_demo.users (
    id bigint PRIMARY KEY,
    organization_id bigint NOT NULL,
    manager_id bigint,
    email text NOT NULL UNIQUE,
    display_name varchar(160) NOT NULL,
    role text NOT NULL DEFAULT 'member'
        CHECK (role IN ('owner', 'admin', 'member', 'viewer', 'guest')),
    bio text,
    avatar bytea,
    preferences jsonb NOT NULL DEFAULT '{}'::jsonb,
    quota numeric(10, 2) NOT NULL DEFAULT 0,
    last_seen_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT CURRENT_TIMESTAMP,
    CONSTRAINT users_organization_fk
        FOREIGN KEY (organization_id)
        REFERENCES dbx_demo.organizations (id)
        ON DELETE CASCADE,
    CONSTRAINT users_manager_fk
        FOREIGN KEY (manager_id)
        REFERENCES dbx_demo.users (id)
        ON DELETE SET NULL
);

CREATE TABLE dbx_demo.tags (
    id bigint PRIMARY KEY,
    name text NOT NULL UNIQUE,
    color text NOT NULL,
    description text,
    created_at timestamptz NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE dbx_demo.projects (
    id bigint PRIMARY KEY,
    organization_id bigint NOT NULL,
    owner_id bigint NOT NULL,
    project_key text NOT NULL UNIQUE,
    name text NOT NULL,
    summary text,
    status text NOT NULL DEFAULT 'active'
        CHECK (status IN ('planned', 'active', 'paused', 'complete', 'archived')),
    priority smallint NOT NULL DEFAULT 3 CHECK (priority BETWEEN 1 AND 5),
    budget numeric(12, 2) NOT NULL DEFAULT 0,
    metadata jsonb NOT NULL DEFAULT '{}'::jsonb,
    archived boolean NOT NULL DEFAULT false,
    start_on date,
    due_on date,
    created_at timestamptz NOT NULL DEFAULT CURRENT_TIMESTAMP,
    CONSTRAINT projects_organization_fk
        FOREIGN KEY (organization_id)
        REFERENCES dbx_demo.organizations (id)
        ON DELETE CASCADE,
    CONSTRAINT projects_owner_fk
        FOREIGN KEY (owner_id)
        REFERENCES dbx_demo.users (id)
        ON DELETE RESTRICT
);

CREATE TABLE dbx_demo.project_members (
    project_id bigint NOT NULL,
    user_id bigint NOT NULL,
    member_role text NOT NULL CHECK (member_role IN ('lead', 'editor', 'viewer', 'observer')),
    joined_at timestamptz NOT NULL DEFAULT CURRENT_TIMESTAMP,
    settings jsonb NOT NULL DEFAULT '{}'::jsonb,
    PRIMARY KEY (project_id, user_id),
    CONSTRAINT project_members_project_fk
        FOREIGN KEY (project_id)
        REFERENCES dbx_demo.projects (id)
        ON DELETE CASCADE,
    CONSTRAINT project_members_user_fk
        FOREIGN KEY (user_id)
        REFERENCES dbx_demo.users (id)
        ON DELETE CASCADE
);

CREATE TABLE dbx_demo.project_assignments (
    project_id bigint NOT NULL,
    user_id bigint NOT NULL,
    assignment_kind text NOT NULL CHECK (assignment_kind IN ('primary', 'backup', 'reviewer')),
    assigned_at timestamptz NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (project_id, user_id),
    CONSTRAINT project_assignments_member_fk
        FOREIGN KEY (project_id, user_id)
        REFERENCES dbx_demo.project_members (project_id, user_id)
        ON DELETE CASCADE
);

CREATE TABLE dbx_demo.project_tags (
    project_id bigint NOT NULL,
    tag_id bigint NOT NULL,
    added_by bigint,
    added_at timestamptz NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (project_id, tag_id),
    CONSTRAINT project_tags_project_fk
        FOREIGN KEY (project_id)
        REFERENCES dbx_demo.projects (id)
        ON DELETE CASCADE,
    CONSTRAINT project_tags_tag_fk
        FOREIGN KEY (tag_id)
        REFERENCES dbx_demo.tags (id)
        ON DELETE CASCADE,
    CONSTRAINT project_tags_added_by_fk
        FOREIGN KEY (added_by)
        REFERENCES dbx_demo.users (id)
        ON DELETE SET NULL
);

CREATE TABLE dbx_demo.tasks (
    id bigint PRIMARY KEY,
    project_id bigint NOT NULL,
    reporter_id bigint NOT NULL,
    assignee_id bigint,
    parent_task_id bigint,
    task_key text NOT NULL UNIQUE,
    status text NOT NULL DEFAULT 'backlog'
        CHECK (status IN ('backlog', 'todo', 'in_progress', 'blocked', 'done', 'cancelled')),
    priority smallint NOT NULL DEFAULT 3 CHECK (priority BETWEEN 1 AND 5),
    title text NOT NULL,
    description text,
    estimate_hours numeric(7, 2) CHECK (estimate_hours >= 0),
    billable boolean NOT NULL DEFAULT false,
    external_id uuid NOT NULL UNIQUE,
    metadata jsonb NOT NULL DEFAULT '{}'::jsonb,
    due_at timestamptz,
    completed_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at timestamptz NOT NULL DEFAULT CURRENT_TIMESTAMP,
    CONSTRAINT tasks_project_fk
        FOREIGN KEY (project_id)
        REFERENCES dbx_demo.projects (id)
        ON DELETE CASCADE,
    CONSTRAINT tasks_reporter_fk
        FOREIGN KEY (reporter_id)
        REFERENCES dbx_demo.users (id)
        ON DELETE RESTRICT,
    CONSTRAINT tasks_assignee_fk
        FOREIGN KEY (assignee_id)
        REFERENCES dbx_demo.users (id)
        ON DELETE SET NULL,
    CONSTRAINT tasks_parent_fk
        FOREIGN KEY (parent_task_id)
        REFERENCES dbx_demo.tasks (id)
        ON DELETE SET NULL
);

CREATE TABLE dbx_demo.task_comments (
    id bigint PRIMARY KEY,
    task_id bigint NOT NULL,
    author_id bigint NOT NULL,
    body text NOT NULL,
    is_internal boolean NOT NULL DEFAULT false,
    metadata jsonb NOT NULL DEFAULT '{}'::jsonb,
    created_at timestamptz NOT NULL DEFAULT CURRENT_TIMESTAMP,
    CONSTRAINT task_comments_task_fk
        FOREIGN KEY (task_id)
        REFERENCES dbx_demo.tasks (id)
        ON DELETE CASCADE,
    CONSTRAINT task_comments_author_fk
        FOREIGN KEY (author_id)
        REFERENCES dbx_demo.users (id)
        ON DELETE RESTRICT
);

CREATE TABLE dbx_demo.task_attachments (
    id bigint PRIMARY KEY,
    task_id bigint NOT NULL,
    uploaded_by bigint NOT NULL,
    file_name text NOT NULL,
    content_type text NOT NULL,
    content bytea NOT NULL,
    checksum text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT CURRENT_TIMESTAMP,
    CONSTRAINT task_attachments_task_fk
        FOREIGN KEY (task_id)
        REFERENCES dbx_demo.tasks (id)
        ON DELETE CASCADE,
    CONSTRAINT task_attachments_uploader_fk
        FOREIGN KEY (uploaded_by)
        REFERENCES dbx_demo.users (id)
        ON DELETE RESTRICT
);

CREATE TABLE dbx_demo.task_dependencies (
    task_id bigint NOT NULL,
    depends_on_task_id bigint NOT NULL,
    relation text NOT NULL CHECK (relation IN ('blocks', 'duplicates', 'relates_to')),
    created_at timestamptz NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (task_id, depends_on_task_id),
    CONSTRAINT task_dependencies_task_fk
        FOREIGN KEY (task_id)
        REFERENCES dbx_demo.tasks (id)
        ON DELETE CASCADE,
    CONSTRAINT task_dependencies_dependency_fk
        FOREIGN KEY (depends_on_task_id)
        REFERENCES dbx_demo.tasks (id)
        ON DELETE CASCADE,
    CONSTRAINT task_dependencies_not_self CHECK (task_id <> depends_on_task_id)
);

CREATE TABLE dbx_demo.time_entries (
    id bigint PRIMARY KEY,
    task_id bigint NOT NULL,
    user_id bigint NOT NULL,
    started_at timestamptz NOT NULL,
    minutes integer NOT NULL CHECK (minutes BETWEEN 1 AND 1440),
    hourly_rate numeric(8, 2) NOT NULL CHECK (hourly_rate >= 0),
    note text,
    billable boolean NOT NULL DEFAULT true,
    CONSTRAINT time_entries_task_fk
        FOREIGN KEY (task_id)
        REFERENCES dbx_demo.tasks (id)
        ON DELETE CASCADE,
    CONSTRAINT time_entries_user_fk
        FOREIGN KEY (user_id)
        REFERENCES dbx_demo.users (id)
        ON DELETE RESTRICT
);

CREATE TABLE dbx_demo.notifications (
    id bigint PRIMARY KEY,
    user_id bigint NOT NULL,
    kind text NOT NULL,
    title text NOT NULL,
    body text NOT NULL,
    payload jsonb NOT NULL DEFAULT '{}'::jsonb,
    read_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT CURRENT_TIMESTAMP,
    CONSTRAINT notifications_user_fk
        FOREIGN KEY (user_id)
        REFERENCES dbx_demo.users (id)
        ON DELETE CASCADE
);

-- Quoted table and column names exercise identifier discovery and quoting.
CREATE TABLE dbx_demo."quoted records" (
    "record id" bigint PRIMARY KEY,
    project_id bigint NOT NULL,
    "display name" text NOT NULL,
    "select" text,
    payload jsonb,
    created_at timestamptz NOT NULL DEFAULT CURRENT_TIMESTAMP,
    CONSTRAINT quoted_records_project_fk
        FOREIGN KEY (project_id)
        REFERENCES dbx_demo.projects (id)
        ON DELETE CASCADE
);

CREATE TABLE dbx_demo."release notes" (
    id bigint PRIMARY KEY,
    project_id bigint NOT NULL,
    author_id bigint NOT NULL,
    "release title" text NOT NULL,
    body text NOT NULL,
    published_at timestamptz,
    CONSTRAINT release_notes_project_fk
        FOREIGN KEY (project_id)
        REFERENCES dbx_demo.projects (id)
        ON DELETE CASCADE,
    CONSTRAINT release_notes_author_fk
        FOREIGN KEY (author_id)
        REFERENCES dbx_demo.users (id)
        ON DELETE RESTRICT
);

CREATE TABLE dbx_support.tickets (
    id bigint PRIMARY KEY,
    organization_id bigint NOT NULL,
    requester_id bigint NOT NULL,
    assignee_id bigint,
    project_id bigint,
    ticket_number text NOT NULL UNIQUE,
    subject text NOT NULL,
    description text NOT NULL,
    status text NOT NULL DEFAULT 'open'
        CHECK (status IN ('open', 'pending', 'waiting_on_customer', 'resolved', 'closed')),
    priority smallint NOT NULL DEFAULT 3 CHECK (priority BETWEEN 1 AND 5),
    labels jsonb NOT NULL DEFAULT '[]'::jsonb,
    opened_at timestamptz NOT NULL DEFAULT CURRENT_TIMESTAMP,
    closed_at timestamptz,
    CONSTRAINT tickets_organization_fk
        FOREIGN KEY (organization_id)
        REFERENCES dbx_demo.organizations (id)
        ON DELETE CASCADE,
    CONSTRAINT tickets_requester_fk
        FOREIGN KEY (requester_id)
        REFERENCES dbx_demo.users (id)
        ON DELETE RESTRICT,
    CONSTRAINT tickets_assignee_fk
        FOREIGN KEY (assignee_id)
        REFERENCES dbx_demo.users (id)
        ON DELETE SET NULL,
    CONSTRAINT tickets_project_fk
        FOREIGN KEY (project_id)
        REFERENCES dbx_demo.projects (id)
        ON DELETE SET NULL
);

CREATE TABLE dbx_support.ticket_messages (
    id bigint PRIMARY KEY,
    ticket_id bigint NOT NULL,
    author_id bigint NOT NULL,
    body text NOT NULL,
    from_customer boolean NOT NULL,
    attachments jsonb NOT NULL DEFAULT '[]'::jsonb,
    created_at timestamptz NOT NULL DEFAULT CURRENT_TIMESTAMP,
    CONSTRAINT ticket_messages_ticket_fk
        FOREIGN KEY (ticket_id)
        REFERENCES dbx_support.tickets (id)
        ON DELETE CASCADE,
    CONSTRAINT ticket_messages_author_fk
        FOREIGN KEY (author_id)
        REFERENCES dbx_demo.users (id)
        ON DELETE RESTRICT
);

CREATE TABLE dbx_analytics.daily_project_metrics (
    metric_date date NOT NULL,
    project_id bigint NOT NULL,
    active_users integer NOT NULL CHECK (active_users >= 0),
    new_tasks integer NOT NULL CHECK (new_tasks >= 0),
    completed_tasks integer NOT NULL CHECK (completed_tasks >= 0),
    hours_logged numeric(10, 2) NOT NULL CHECK (hours_logged >= 0),
    payload jsonb NOT NULL DEFAULT '{}'::jsonb,
    PRIMARY KEY (metric_date, project_id),
    CONSTRAINT daily_metrics_project_fk
        FOREIGN KEY (project_id)
        REFERENCES dbx_demo.projects (id)
        ON DELETE CASCADE
);

CREATE TABLE dbx_analytics.task_status_history (
    id bigint PRIMARY KEY,
    task_id bigint NOT NULL,
    changed_by bigint NOT NULL,
    from_status text,
    to_status text NOT NULL,
    reason text,
    metadata jsonb NOT NULL DEFAULT '{}'::jsonb,
    changed_at timestamptz NOT NULL DEFAULT CURRENT_TIMESTAMP,
    CONSTRAINT status_history_task_fk
        FOREIGN KEY (task_id)
        REFERENCES dbx_demo.tasks (id)
        ON DELETE CASCADE,
    CONSTRAINT status_history_user_fk
        FOREIGN KEY (changed_by)
        REFERENCES dbx_demo.users (id)
        ON DELETE RESTRICT
);

CREATE TABLE dbx_analytics.audit_log (
    event_id uuid PRIMARY KEY,
    actor_id bigint,
    action text NOT NULL,
    resource_type text NOT NULL,
    resource_id text NOT NULL,
    details jsonb NOT NULL DEFAULT '{}'::jsonb,
    occurred_at timestamptz NOT NULL DEFAULT CURRENT_TIMESTAMP,
    CONSTRAINT audit_log_actor_fk
        FOREIGN KEY (actor_id)
        REFERENCES dbx_demo.users (id)
        ON DELETE SET NULL
);

CREATE TABLE dbx_analytics.query_runs (
    id bigint PRIMARY KEY,
    user_id bigint NOT NULL,
    query_name text NOT NULL,
    sql_text text NOT NULL,
    duration_ms integer NOT NULL CHECK (duration_ms >= 0),
    rows_returned integer NOT NULL CHECK (rows_returned >= 0),
    succeeded boolean NOT NULL,
    error_message text,
    parameters jsonb NOT NULL DEFAULT '{}'::jsonb,
    executed_at timestamptz NOT NULL DEFAULT CURRENT_TIMESTAMP,
    CONSTRAINT query_runs_user_fk
        FOREIGN KEY (user_id)
        REFERENCES dbx_demo.users (id)
        ON DELETE CASCADE
);

-- Views give the app read-only surfaces in addition to ordinary tables.
CREATE OR REPLACE VIEW dbx_demo.project_overview AS
SELECT
    p.id,
    p.project_key,
    p.name,
    p.status,
    p.priority,
    o.slug AS organization_slug,
    u.display_name AS owner_name,
    COUNT(DISTINCT t.id) AS task_count,
    COUNT(DISTINCT t.id) FILTER (WHERE t.status = 'done') AS completed_task_count,
    COALESCE(SUM(te.minutes), 0) AS minutes_logged,
    p.created_at
FROM dbx_demo.projects AS p
JOIN dbx_demo.organizations AS o ON o.id = p.organization_id
JOIN dbx_demo.users AS u ON u.id = p.owner_id
LEFT JOIN dbx_demo.tasks AS t ON t.project_id = p.id
LEFT JOIN dbx_demo.time_entries AS te ON te.task_id = t.id
GROUP BY p.id, p.project_key, p.name, p.status, p.priority, o.slug, u.display_name, p.created_at;

CREATE OR REPLACE VIEW dbx_support.open_ticket_queue AS
SELECT
    t.id,
    t.ticket_number,
    t.status,
    t.priority,
    t.subject,
    o.name AS organization_name,
    requester.display_name AS requester_name,
    assignee.display_name AS assignee_name,
    t.opened_at
FROM dbx_support.tickets AS t
JOIN dbx_demo.organizations AS o ON o.id = t.organization_id
JOIN dbx_demo.users AS requester ON requester.id = t.requester_id
LEFT JOIN dbx_demo.users AS assignee ON assignee.id = t.assignee_id
WHERE t.status NOT IN ('resolved', 'closed');

CREATE OR REPLACE VIEW dbx_analytics.project_health AS
SELECT
    p.id AS project_id,
    p.project_key,
    p.name,
    p.status,
    COALESCE(SUM(m.new_tasks), 0) AS new_tasks_last_45_days,
    COALESCE(SUM(m.completed_tasks), 0) AS completed_tasks_last_45_days,
    COALESCE(SUM(m.hours_logged), 0)::numeric(12, 2) AS hours_logged_last_45_days
FROM dbx_demo.projects AS p
LEFT JOIN dbx_analytics.daily_project_metrics AS m ON m.project_id = p.id
GROUP BY p.id, p.project_key, p.name, p.status;

-- Indexes are part of the fixture's schema and make larger browsing/querying
-- sessions feel like a realistic application database.
CREATE INDEX projects_organization_status_idx
    ON dbx_demo.projects (organization_id, status);
CREATE INDEX projects_owner_idx
    ON dbx_demo.projects (owner_id);
CREATE INDEX tasks_project_status_idx
    ON dbx_demo.tasks (project_id, status, priority);
CREATE INDEX tasks_assignee_idx
    ON dbx_demo.tasks (assignee_id, updated_at DESC);
CREATE INDEX task_comments_task_created_idx
    ON dbx_demo.task_comments (task_id, created_at DESC);
CREATE INDEX task_dependencies_created_idx
    ON dbx_demo.task_dependencies (task_id, created_at DESC);
CREATE INDEX time_entries_task_started_idx
    ON dbx_demo.time_entries (task_id, started_at DESC);
CREATE INDEX tickets_queue_idx
    ON dbx_support.tickets (status, priority, opened_at DESC);
CREATE INDEX ticket_messages_ticket_created_idx
    ON dbx_support.ticket_messages (ticket_id, created_at);
CREATE INDEX daily_metrics_project_date_idx
    ON dbx_analytics.daily_project_metrics (project_id, metric_date DESC);
CREATE INDEX audit_log_resource_idx
    ON dbx_analytics.audit_log (resource_type, resource_id, occurred_at DESC);

-- ---------------------------------------------------------------------------
-- Data
-- ---------------------------------------------------------------------------

INSERT INTO dbx_demo.organizations
    (id, slug, name, plan, active, settings, monthly_budget, created_at)
SELECT
    s,
    'org-' || lpad(s::text, 3, '0'),
    CASE
        WHEN s = 1 THEN 'O''Reilly & Sons — North'
        WHEN s = 2 THEN '永續資料实验室'
        ELSE 'Organization ' || s
    END,
    (ARRAY['free', 'team', 'business', 'enterprise'])[1 + ((s - 1) % 4)],
    s % 13 <> 0,
    jsonb_build_object(
        'region', (ARRAY['eu-west', 'us-east', 'ap-south'])[1 + ((s - 1) % 3)],
        'seat_limit', 10 + s * 4,
        'features', jsonb_build_array('filters', s % 2 = 0, 'audit', s % 3 = 0)
    ),
    round((12500 + s * 731.25)::numeric, 2),
    CURRENT_TIMESTAMP - ((s % 900)::text || ' days')::interval
FROM generate_series(1, 25) AS series(s);

INSERT INTO dbx_demo.users
    (id, organization_id, manager_id, email, display_name, role, bio, avatar,
     preferences, quota, last_seen_at, created_at)
SELECT
    s,
    ((s - 1) % 25) + 1,
    CASE WHEN s % 32 = 1 THEN NULL ELSE ((s - 1) / 32) * 32 + 1 END,
    'user' || lpad(s::text, 4, '0') || '@example.test',
    CASE
        WHEN s = 1 THEN 'Ada Lovelace'
        WHEN s = 2 THEN 'Grace Hopper'
        WHEN s = 3 THEN 'Linus Torvalds'
        ELSE 'Demo User ' || lpad(s::text, 4, '0')
    END,
    CASE
        WHEN s % 32 = 1 THEN 'owner'
        WHEN s % 11 = 0 THEN 'admin'
        WHEN s % 7 = 0 THEN 'viewer'
        WHEN s % 17 = 0 THEN 'guest'
        ELSE 'member'
    END,
    CASE
        WHEN s = 1 THEN 'Mathematician, writer, and first programmer.'
        WHEN s = 2 THEN 'Pioneer of compiler design and human-machine collaboration.'
        WHEN s = 3 THEN 'Maintains a distributed systems project.'
        WHEN s = 4 THEN 'Line one' || chr(10) || 'Line two with an apostrophe: O''Reilly'
        WHEN s % 23 = 0 THEN ''
        WHEN s % 19 = 0 THEN NULL
        ELSE 'A deliberately varied biography for search and detail-panel testing.'
    END,
    CASE WHEN s % 17 = 0 THEN decode(md5('avatar-' || s), 'hex') ELSE NULL END,
    jsonb_build_object(
        'theme', (ARRAY['dark', 'light', 'system'])[1 + ((s - 1) % 3)],
        'compact_mode', s % 2 = 0,
        'saved_views', jsonb_build_array('mine', 'recent', 'priority-' || (s % 5))
    ),
    round((s * 17.35)::numeric, 2),
    CASE WHEN s % 13 = 0 THEN NULL
         ELSE CURRENT_TIMESTAMP - ((s % 90)::text || ' days')::interval END,
    CURRENT_TIMESTAMP - ((s % 720)::text || ' days')::interval
FROM generate_series(1, 800) AS series(s);

INSERT INTO dbx_demo.tags (id, name, color, description, created_at)
SELECT
    s,
    'tag-' || lpad(s::text, 2, '0'),
    '#' || lpad(to_hex((s * 7919) % 16777215), 6, '0'),
    CASE WHEN s % 9 = 0 THEN NULL ELSE 'Reusable label number ' || s END,
    CURRENT_TIMESTAMP - ((s % 300)::text || ' days')::interval
FROM generate_series(1, 40) AS series(s);

INSERT INTO dbx_demo.projects
    (id, organization_id, owner_id, project_key, name, summary, status, priority,
     budget, metadata, archived, start_on, due_on, created_at)
WITH generated AS (
    SELECT
        s,
        ((s - 1) % 25) + 1 AS organization_id,
        ((s * 17 - 1) % 800) + 1 AS owner_id,
        (ARRAY['planned', 'active', 'paused', 'complete', 'archived'])
            [1 + ((s - 1) % 5)] AS status
    FROM generate_series(1, 220) AS series(s)
)
SELECT
    s,
    organization_id,
    owner_id,
    'DBX-' || lpad(s::text, 4, '0'),
    CASE WHEN s = 1 THEN 'The Export Observatory' ELSE 'Project ' || lpad(s::text, 4, '0') END,
    CASE WHEN s % 17 = 0 THEN NULL
         ELSE 'A realistic project summary with enough text to inspect in the app.' END,
    status,
    1 + (s % 5),
    round((s * 4825.75)::numeric, 2),
    jsonb_build_object(
        'team_size', 2 + (s % 12),
        'risk', (ARRAY['low', 'medium', 'high'])[1 + ((s - 1) % 3)],
        'milestones', jsonb_build_array('discovery', 'build', 'launch')
    ),
    status = 'archived',
    CURRENT_DATE - (s % 500),
    CURRENT_DATE + (s % 180),
    CURRENT_TIMESTAMP - ((s % 500)::text || ' days')::interval
FROM generated;

INSERT INTO dbx_demo.project_members
    (project_id, user_id, member_role, joined_at, settings)
SELECT
    p.id,
    ((p.owner_id + member.slot - 1) % 800) + 1,
    CASE member.slot
        WHEN 0 THEN 'lead'
        WHEN 1 THEN 'editor'
        WHEN 2 THEN 'viewer'
        ELSE 'observer'
    END,
    CURRENT_TIMESTAMP - (((p.id + member.slot) % 300)::text || ' days')::interval,
    jsonb_build_object('notifications', member.slot <> 3, 'allocation', 25 * (member.slot + 1))
FROM dbx_demo.projects AS p
CROSS JOIN generate_series(0, 3) AS member(slot)
ON CONFLICT DO NOTHING;

INSERT INTO dbx_demo.project_assignments
    (project_id, user_id, assignment_kind, assigned_at)
SELECT
    project_id,
    user_id,
    CASE WHEN user_id % 2 = 0 THEN 'primary'
         WHEN user_id % 3 = 0 THEN 'backup'
         ELSE 'reviewer' END,
    CURRENT_TIMESTAMP - ((project_id % 120)::text || ' days')::interval
FROM dbx_demo.project_members
WHERE user_id % 3 <> 1;

INSERT INTO dbx_demo.project_tags (project_id, tag_id, added_by, added_at)
SELECT
    p.id,
    ((p.id + tag_slot.slot) % 40) + 1,
    p.owner_id,
    CURRENT_TIMESTAMP - ((p.id % 200)::text || ' days')::interval
FROM dbx_demo.projects AS p
CROSS JOIN generate_series(0, 2) AS tag_slot(slot);

INSERT INTO dbx_demo.tasks
    (id, project_id, reporter_id, assignee_id, parent_task_id, task_key, status,
     priority, title, description, estimate_hours, billable, external_id, metadata,
     due_at, completed_at, created_at, updated_at)
WITH generated AS (
    SELECT
        s,
        ((s - 1) % 220) + 1 AS project_id,
        ((s * 13 - 1) % 800) + 1 AS reporter_id,
        CASE WHEN s % 9 = 0 THEN NULL ELSE ((s * 29 - 1) % 800) + 1 END AS assignee_id,
        CASE WHEN s > 1 AND s % 15 <> 0 THEN s - 1 ELSE NULL END AS parent_task_id,
        (ARRAY['backlog', 'todo', 'in_progress', 'blocked', 'done', 'cancelled'])
            [1 + ((s - 1) % 6)] AS status
    FROM generate_series(1, 6000) AS series(s)
)
SELECT
    s,
    project_id,
    reporter_id,
    assignee_id,
    parent_task_id,
    'TASK-' || lpad(s::text, 6, '0'),
    status,
    1 + (s % 5),
    CASE WHEN s = 1 THEN 'Make the database dump tell a story'
         ELSE 'Task ' || lpad(s::text, 6, '0') || ' — investigate the next useful slice' END,
    CASE WHEN s % 29 = 0 THEN NULL
         WHEN s % 31 = 0 THEN 'Contains a quote: O''Reilly; and a newline' || chr(10) || 'for editor testing.'
         ELSE 'Generated task description with searchable words: export, schema, filter, relation.' END,
    round(((s % 80) + 1.25)::numeric, 2),
    s % 4 <> 0,
    md5('dbx-task-' || s)::uuid,
    jsonb_build_object(
        'sprint', 1 + (s % 12),
        'source', (ARRAY['import', 'api', 'manual', 'automation'])[1 + ((s - 1) % 4)],
        'flags', jsonb_build_array(s % 2 = 0, s % 5 = 0, s % 11 = 0)
    ),
    CURRENT_TIMESTAMP + ((s % 180)::text || ' days')::interval,
    CASE WHEN status IN ('done', 'cancelled')
         THEN CURRENT_TIMESTAMP - ((s % 300)::text || ' hours')::interval END,
    CURRENT_TIMESTAMP - ((s % 365)::text || ' days')::interval,
    CURRENT_TIMESTAMP - ((s % 120)::text || ' hours')::interval
FROM generated;

INSERT INTO dbx_demo.task_comments
    (id, task_id, author_id, body, is_internal, metadata, created_at)
SELECT
    s,
    ((s - 1) % 6000) + 1,
    ((s * 5 - 1) % 800) + 1,
    CASE
        WHEN s = 1 THEN 'First comment: this is the happy path.'
        WHEN s = 2 THEN 'A multiline note' || chr(10) || 'with an apostrophe: O''Reilly'
        WHEN s % 19 = 0 THEN ''
        ELSE format('Comment %s: reviewed the current state and left a useful breadcrumb.', s)
    END,
    s % 7 = 0,
    jsonb_build_object('mentions', jsonb_build_array('user' || (s % 40)), 'edited', s % 13 = 0),
    CURRENT_TIMESTAMP - ((s % 365)::text || ' days')::interval
FROM generate_series(1, 15000) AS series(s);

INSERT INTO dbx_demo.task_attachments
    (id, task_id, uploaded_by, file_name, content_type, content, checksum, created_at)
SELECT
    s,
    ((s - 1) % 6000) + 1,
    ((s * 11 - 1) % 800) + 1,
    CASE WHEN s % 10 = 0 THEN 'design review ✓ ' || s || '.png'
         ELSE 'attachment-' || lpad(s::text, 5, '0') || '.bin' END,
    CASE WHEN s % 3 = 0 THEN 'application/pdf'
         WHEN s % 3 = 1 THEN 'image/png'
         ELSE 'application/octet-stream' END,
    decode(md5('attachment-' || s) || md5('payload-' || s), 'hex'),
    md5('attachment-' || s || '-checksum'),
    CURRENT_TIMESTAMP - ((s % 300)::text || ' days')::interval
FROM generate_series(1, 2000) AS series(s);

INSERT INTO dbx_demo.task_dependencies
    (task_id, depends_on_task_id, relation, created_at)
SELECT
    s + 1,
    s,
    CASE WHEN s % 3 = 0 THEN 'blocks'
         WHEN s % 3 = 1 THEN 'relates_to'
         ELSE 'duplicates' END,
    CURRENT_TIMESTAMP - ((s % 180)::text || ' days')::interval
FROM generate_series(1, 5000) AS series(s);

INSERT INTO dbx_demo.time_entries
    (id, task_id, user_id, started_at, minutes, hourly_rate, note, billable)
SELECT
    s,
    ((s - 1) % 6000) + 1,
    ((s * 7 - 1) % 800) + 1,
    CURRENT_TIMESTAMP - ((s % 180)::text || ' days')::interval,
    15 + (s % 240),
    round((45 + (s % 9) * 7.5)::numeric, 2),
    CASE WHEN s % 17 = 0 THEN NULL ELSE 'Work session ' || s || ' — implementation and review' END,
    s % 8 <> 0
FROM generate_series(1, 12000) AS series(s);

INSERT INTO dbx_demo.notifications
    (id, user_id, kind, title, body, payload, read_at, created_at)
SELECT
    s,
    ((s * 3 - 1) % 800) + 1,
    (ARRAY['assignment', 'mention', 'status_change', 'digest'])[1 + ((s - 1) % 4)],
    'Notification ' || s,
    CASE WHEN s % 23 = 0 THEN 'You have a new high-priority task.'
         ELSE 'A generated event needs your attention in the demo workspace.' END,
    jsonb_build_object('task_id', ((s - 1) % 6000) + 1, 'urgent', s % 23 = 0),
    CASE WHEN s % 4 = 0 THEN CURRENT_TIMESTAMP - ((s % 30)::text || ' days')::interval END,
    CURRENT_TIMESTAMP - ((s % 90)::text || ' days')::interval
FROM generate_series(1, 5000) AS series(s);

INSERT INTO dbx_demo."quoted records"
    ("record id", project_id, "display name", "select", payload, created_at)
SELECT
    s,
    ((s - 1) % 220) + 1,
    CASE WHEN s = 1 THEN 'A record with a quoted identifier' ELSE 'Display row ' || s END,
    CASE WHEN s % 5 = 0 THEN NULL ELSE 'value-' || s END,
    jsonb_build_object('ordinal', s, 'special', s = 1, 'empty', s % 7 = 0),
    CURRENT_TIMESTAMP - ((s % 100)::text || ' days')::interval
FROM generate_series(1, 100) AS series(s);

INSERT INTO dbx_demo."release notes"
    (id, project_id, author_id, "release title", body, published_at)
SELECT
    s,
    ((s - 1) % 220) + 1,
    ((s * 19 - 1) % 800) + 1,
    CASE WHEN s = 1 THEN '0.1.0 — First preview' ELSE 'Release note ' || s END,
    CASE WHEN s % 12 = 0 THEN 'Draft notes are intentionally long enough to wrap in the detail view.'
         ELSE 'A concise release note for project ' || (((s - 1) % 220) + 1) END,
    CASE WHEN s % 6 = 0 THEN NULL ELSE CURRENT_TIMESTAMP - ((s % 240)::text || ' days')::interval END
FROM generate_series(1, 400) AS series(s);

INSERT INTO dbx_support.tickets
    (id, organization_id, requester_id, assignee_id, project_id, ticket_number,
     subject, description, status, priority, labels, opened_at, closed_at)
WITH generated AS (
    SELECT
        s,
        ((s - 1) % 25) + 1 AS organization_id,
        ((s * 23 - 1) % 800) + 1 AS requester_id,
        CASE WHEN s % 8 = 0 THEN NULL ELSE ((s * 31 - 1) % 800) + 1 END AS assignee_id,
        ((s - 1) % 220) + 1 AS project_id,
        (ARRAY['open', 'pending', 'waiting_on_customer', 'resolved', 'closed'])
            [1 + ((s - 1) % 5)] AS status
    FROM generate_series(1, 3000) AS series(s)
)
SELECT
    s,
    organization_id,
    requester_id,
    assignee_id,
    project_id,
    'SUP-' || lpad(s::text, 6, '0'),
    CASE WHEN s = 1 THEN 'Export contains a foreign-key surprise'
         ELSE 'Support request ' || s || ': investigate workspace behavior' END,
    CASE WHEN s % 27 = 0 THEN 'A long support description with enough content for scrolling and selection.'
         ELSE 'Customer-reported issue for the DBX demo environment.' END,
    status,
    1 + (s % 5),
    jsonb_build_array((ARRAY['billing', 'access', 'performance', 'import', 'ui'])[1 + ((s - 1) % 5)], 'generated'),
    CURRENT_TIMESTAMP - ((s % 300)::text || ' days')::interval,
    CASE WHEN status IN ('resolved', 'closed')
         THEN CURRENT_TIMESTAMP - ((s % 120)::text || ' days')::interval END
FROM generated;

INSERT INTO dbx_support.ticket_messages
    (id, ticket_id, author_id, body, from_customer, attachments, created_at)
SELECT
    s,
    ((s - 1) % 3000) + 1,
    ((s * 37 - 1) % 800) + 1,
    CASE WHEN s % 41 = 0 THEN 'Message with a newline' || chr(10) || 'and a quoted phrase: "please help".'
         ELSE format('Ticket message %s with a useful support trail.', s) END,
    s % 2 = 0,
    CASE WHEN s % 10 = 0
         THEN jsonb_build_array(jsonb_build_object('name', 'screen-' || s || '.png', 'size', 1024 + s))
         ELSE '[]'::jsonb END,
    CURRENT_TIMESTAMP - ((s % 300)::text || ' days')::interval
FROM generate_series(1, 12000) AS series(s);

INSERT INTO dbx_analytics.daily_project_metrics
    (metric_date, project_id, active_users, new_tasks, completed_tasks, hours_logged, payload)
SELECT
    CURRENT_DATE - days.day_offset,
    p.id,
    5 + ((p.id + days.day_offset) % 80),
    (p.id + days.day_offset * 3) % 18,
    (p.id * 2 + days.day_offset) % 15,
    round(((p.id % 20) + days.day_offset % 11 + 0.5)::numeric, 2),
    jsonb_build_object('source', 'fixture', 'window', 'daily', 'day_offset', days.day_offset)
FROM generate_series(0, 44) AS days(day_offset)
CROSS JOIN dbx_demo.projects AS p;

INSERT INTO dbx_analytics.task_status_history
    (id, task_id, changed_by, from_status, to_status, reason, metadata, changed_at)
SELECT
    s,
    ((s - 1) % 6000) + 1,
    ((s * 43 - 1) % 800) + 1,
    CASE WHEN s % 6 = 0 THEN NULL
         ELSE (ARRAY['backlog', 'todo', 'in_progress', 'blocked', 'done'])[1 + ((s - 1) % 5)] END,
    (ARRAY['backlog', 'todo', 'in_progress', 'blocked', 'done', 'cancelled'])
        [1 + ((s + 1) % 6)],
    CASE WHEN s % 13 = 0 THEN NULL ELSE 'Generated workflow transition' END,
    jsonb_build_object('automated', s % 4 = 0, 'source_event', 'fixture-' || s),
    CURRENT_TIMESTAMP - ((s % 365)::text || ' days')::interval
FROM generate_series(1, 18000) AS series(s);

INSERT INTO dbx_analytics.audit_log
    (event_id, actor_id, action, resource_type, resource_id, details, occurred_at)
SELECT
    md5('audit-' || s)::uuid,
    CASE WHEN s % 10 = 0 THEN NULL ELSE ((s * 47 - 1) % 800) + 1 END,
    (ARRAY['created', 'updated', 'deleted', 'exported', 'imported'])[1 + ((s - 1) % 5)],
    (ARRAY['task', 'project', 'ticket', 'user'])[1 + ((s - 1) % 4)],
    ((s - 1) % 6000 + 1)::text,
    jsonb_build_object('request_id', md5('request-' || s), 'ip', '192.0.2.' || (1 + (s % 240))),
    CURRENT_TIMESTAMP - ((s % 500)::text || ' days')::interval
FROM generate_series(1, 16000) AS series(s);

INSERT INTO dbx_analytics.query_runs
    (id, user_id, query_name, sql_text, duration_ms, rows_returned, succeeded,
     error_message, parameters, executed_at)
SELECT
    s,
    ((s * 53 - 1) % 800) + 1,
    (ARRAY['task_search', 'project_rollup', 'ticket_queue', 'recent_activity'])
        [1 + ((s - 1) % 4)],
    CASE WHEN s % 17 = 0
         THEN 'SELECT id, project_id, status FROM dbx_demo.tasks WHERE status = $1 ORDER BY updated_at DESC'
         ELSE 'SELECT * FROM dbx_demo.project_overview WHERE status <> ''archived''' END,
    4 + (s % 2200),
    (s * 13) % 5000,
    s % 29 <> 0,
    CASE WHEN s % 29 = 0 THEN 'Synthetic timeout for error-state testing' END,
    jsonb_build_object('status', (ARRAY['todo', 'in_progress', 'done'])[1 + ((s - 1) % 3)], 'limit', 50),
    CURRENT_TIMESTAMP - ((s % 180)::text || ' days')::interval
FROM generate_series(1, 2000) AS series(s);

-- Refresh planner statistics so the fixture behaves like an actively used DB.
ANALYZE dbx_demo.organizations;
ANALYZE dbx_demo.users;
ANALYZE dbx_demo.projects;
ANALYZE dbx_demo.tasks;
ANALYZE dbx_demo.task_comments;
ANALYZE dbx_support.tickets;
ANALYZE dbx_analytics.daily_project_metrics;
ANALYZE dbx_analytics.audit_log;
