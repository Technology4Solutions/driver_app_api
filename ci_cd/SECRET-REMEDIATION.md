# Exposed credentials remediation

The committed root `.env` and load-test `credentials.local.sh` are compromised.
Deleting them does not revoke credentials or clean existing clones.

## Runtime

Local Docker and Cargo runs read the private, ignored `.env`. Copy `.env.example`
to `.env`, fill in local settings and protect it with `chmod 600 .env`.
Never force-add this file to Git. Deployment continues to read Vault.

Vault deployment defaults and bundle keys are documented in README.md.
Local settings must configure reachable DB, RABBITMQ_URL and REDIS_URL endpoints;
the deployment defaults use Swarm service names.

## Rotation (requires infrastructure access)

1. Identify every environment and service using the exposed values, including
   shared Java services and broadcasts. Inventory all historical values privately.
2. Replace the database identity/password at the database authority, retain only
   required grants, update Vault DB_* or SCHOOL_BUS_DATASOURCE_* keys, redeploy
   consumers, verify access, and revoke the old database identity/password.
3. Replace API username and password in Vault and every API client/broadcast
   consumer. Remove REAXIUM_API_PASSWORD_PREVIOUS and any basic-auth aliases
   that retain exposed credentials. Restart all replicas; reject old credentials.
4. Generate a new JWT signing secret with at least 32 random bytes directly into
   Vault using an approved secret workflow. Restart all replicas and require
   clients to authenticate again. Verify previously issued JWTs are rejected.
5. Check access logs and invalidate any other credentials found in historical
   copies of these files. Do not paste secrets into tickets, terminals or logs.

## History cleanup

A rewritten mirror removes both paths from every local branch and tag. Before
publishing it, freeze pushes, verify all remote refs were fetched, and scan the
rewritten history for copied secret values in other paths. Publish rewritten refs
with coordinated force pushes. Remove server pull-request/cached refs using the
hosting provider procedure. Collaborators must re-clone; old clones, forks,
archives and backups still contain the exposed material. Rotation is mandatory.

Do not push a mirror containing refs/original. Reflogs and unreachable objects in
old clones also retain the secrets until those clones are deleted or cleaned.
