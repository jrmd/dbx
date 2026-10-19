# MongoDB and Redis editing

Select a row in a MongoDB collection and choose **Edit document**. The editor fetches the complete native document by `_id` as canonical Extended JSON, so values such as ObjectId, Int64, Decimal128, dates, nested objects and binary data retain their BSON types. Keep `_id` unchanged. Choose **Review changes**, then **Apply changes** to replace that document. The complete original BSON document participates in the write predicate; if another client adds, changes or removes any field, reload before editing again. The editor does not upsert.

Select a Redis key and choose **Edit key value**. The JSON shape is:

| Type | Editable value |
| --- | --- |
| String | `{"type":"string","value":"text"}` |
| Hash | `{"type":"hash","value":[["field","text"]]}` |
| List | `{"type":"list","value":["first","second"]}` |
| Set | `{"type":"set","value":["member"]}` |
| Sorted set | `{"type":"zset","value":[["member","1.25"]]}` |

Sorted-set scores are finite numeric strings. List order is preserved. Empty hashes, lists, sets and sorted sets remove the key, matching Redis's empty-collection behavior. Keys and values in this editor must be UTF-8; raw Redis commands remain available for binary data.

Streams display their first 1,000 entries in a read-only `entries` array. Add new entries in `append` as arrays of `[field,value]` pairs, and add existing displayed entry IDs to `delete`. Append operations use Redis-generated IDs; editing an existing ID in place is unsupported. For example:

```json
{
  "type": "stream",
  "entries": [{"id":"1720000000000-0","fields":[["event","opened"]]}],
  "append": [[["event","closed"]]],
  "delete": ["1720000000000-0"]
}
```

The stream's `entries` array must remain exactly as loaded. Review shows the full proposed operations before applying them.

Every Redis save validates the document before mutation and uses an atomic script to compare the key's binary snapshot before writing. Concurrent changes, expiration, and switching logical databases invalidate the edit. Current TTL is preserved; missing/expired keys are never recreated by a stale edit. The editor allows at most 10,000 collection elements, strings up to 1 MiB, and keys using at most 4 MiB of Redis memory. It requires access to `MEMORY USAGE`, `DUMP`, and scripting commands. Server-side resource failures follow Redis scripting semantics.

Protected connections disable the edit buttons and reject direct structured writes. Closing a connection, locking the vault, replacing its engine, changing databases or closing the source table invalidates the editor and releases its document/key snapshot.
