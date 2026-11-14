# Backing up and restoring a CraftCloud server

A CraftCloud server keeps everything in two places, and a backup needs **both, taken together**:

| What | Where (Docker Compose) | Where (NixOS module) |
|---|---|---|
| The database: accounts, folders, file names, shares, trash, versions' records | the `pg_data` volume (PostgreSQL) | NixOS's PostgreSQL, database `craftcloud` |
| The file contents and CraftCloud's settings (`.craftcloud/settings.env`) | the `storage` volume (`/app/storage`) | `services.craftcloud.dataDir` (default `/var/lib/craftcloud`) |

Keep your `.env` (or the NixOS environment file) with the backup too: it holds the database password, and without
the matching settings a restored server won't start the same way.

The database says which stored blob belongs to which file, so a database from Monday and files from Tuesday don't
fit together. Stop CraftCloud (not PostgreSQL) for the minute a backup takes, so nothing changes in between. The
desktop clients wait and carry on when it's back.

## Docker Compose

Run these in the folder with `compose.yaml`. The files land in the current folder; copy them somewhere else
afterwards (another disk, another machine).

**Back up**

```sh
day=$(date +%F)
docker compose stop craftcloud
docker compose exec -T postgres pg_dump -U craftcloud -Fc craftcloud > "craftcloud-db-$day.dump"
docker run --rm --volumes-from "$(docker compose ps -aq craftcloud)" -v "$PWD:/backup" alpine \
  tar -C /app/storage -czf "/backup/craftcloud-storage-$day.tar.gz" .
docker compose start craftcloud
```

**Restore** (onto the same or a new machine with the same `.env`; this replaces what the server has now)

```sh
day=2026-11-14   # the backup to restore
docker compose up -d --wait postgres
docker compose stop craftcloud 2>/dev/null; docker compose create craftcloud
docker compose exec -T postgres dropdb -U craftcloud --if-exists craftcloud
docker compose exec -T postgres createdb -U craftcloud craftcloud
docker compose exec -T postgres pg_restore -U craftcloud -d craftcloud --no-owner < "craftcloud-db-$day.dump"
docker run --rm --volumes-from "$(docker compose ps -aq craftcloud)" -v "$PWD:/backup:ro" alpine \
  sh -c 'find /app/storage -mindepth 1 -delete && tar -C /app/storage -xzf "/backup/craftcloud-storage-'"$day"'.tar.gz"'
docker compose up -d
```

Both were tested end to end on 2026-11-14 (back up, wipe the volumes, restore: same account, same file).
Then sign in on the web page and open a few files. The desktop clients reconnect by themselves. Files changed on a
computer after the backup was taken are uploaded again by its client; anything changed only in the cloud after the
backup is gone.

## NixOS

The same steps for the NixOS module. Unlike the Compose steps above, these haven't been run end to end yet.

With the NixOS module, as root (the container service is `docker-craftcloud` or `podman-craftcloud`, after
`virtualisation.oci-containers.backend`):

```sh
day=$(date +%F)
systemctl stop docker-craftcloud
sudo -u postgres pg_dump -Fc craftcloud > "/root/craftcloud-db-$day.dump"
tar -C /var/lib/craftcloud -czf "/root/craftcloud-storage-$day.tar.gz" .
systemctl start docker-craftcloud
```

To restore: stop `docker-craftcloud`, then `sudo -u postgres dropdb craftcloud`,
`sudo -u postgres createdb -O craftcloud craftcloud`,
`sudo -u postgres pg_restore -d craftcloud --no-owner --role=craftcloud <dump>`, empty `dataDir` and unpack the
storage archive into it (`tar -C /var/lib/craftcloud -xzf …`), and start the service. Tools that back up whole
folders (restic, borg) work too, as long as they take a `pg_dump` and `dataDir` while the service is stopped.

## Before an update

Take a backup before updating to a new release, and read its [release notes](RELEASE-NOTES.md) first. To go back,
run the previous image (`CRAFTCLOUD_IMAGE=ghcr.io/jonasbove/craftcloud:<previous version>`) and restore the backup:
a newer server may have changed the database in ways an older one can't read.
