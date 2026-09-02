# RBAC — Turing Tarpit

## Roles (org)

| Role | Rank | Notes |
|------|------|--------|
| **owner** | 40 | Full control; only role that can transfer ownership |
| **admin** | 30 | Manage members, policies, compute destroy, dangerous agents |
| **billing** | 20 | View/manage billing & invoices only |
| **member** | 10 | Day-to-day: workloads create/start/stop, read secrets, safe agents |

## Permissions matrix

| Permission | owner | admin | billing | member |
|------------|:-----:|:-----:|:-------:|:------:|
| view org / members | ✓ | ✓ | ✓ | ✓ |
| invite / remove / change roles | ✓ | ✓ | | |
| transfer ownership | ✓ | | | |
| view / manage billing | ✓ | ✓ | ✓ | |
| SSO / policies / settings | ✓ | ✓ | | |
| create workspace | ✓ | ✓ | | |
| create / start / stop workload | ✓ | ✓ | | ✓ |
| **destroy** workload / quotas | ✓ | ✓ | | |
| read secrets | ✓ | ✓ | | ✓ |
| write secrets | ✓ | ✓ | | |
| run agent (safe) | ✓ | ✓ | | ✓ |
| run agent (**dangerous**) | ✓ | ✓ | | |

Single source: `OrgRole::allows(OrgPermission)` in `tt-core`.

## Hard rules

1. **Suspended seat** → no mutations (`require_permission` / `require_active_member`).
2. **Last owner** cannot be demoted or removed.
3. Cannot grant a role **higher** than your own.
4. Only **owner** can promote to owner or demote another owner.
5. Org **billing status** suspended/unpaid blocks compute create/start and git push (see ADMIN.md).
6. **Superadmin** (`SUPERADMIN_IDS`) is platform-wide, not an org role.

## HTTP

```http
POST   /api/orgs/:slug/members/:principal_id/role   { "role": "admin" }
DELETE /api/orgs/:slug/members/:principal_id
POST   /api/orgs/:slug/transfer-ownership           { "new_owner_id": "..." }
```

Service: `OrgService::require_permission`, `set_member_role`, `remove_member`, `transfer_ownership`.
