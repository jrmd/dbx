-- MySQL 8.4 binary/UUID manual QA fixtures.
-- Select the target database before running this script. All objects are local
-- to that database. Safe to rerun: existing rows (including manual edits) stay intact.
-- Reference columns record the ORIGINAL seeded value; they intentionally do not
-- change when you edit a binary cell. Use a fresh database to reset the fixtures.
-- UUID standard uses UUID_TO_BIN(uuid, 0); swapped uses UUID_TO_BIN(uuid, 1).
-- The swap is explicit: never auto-detect it from a UUID version nibble.

CREATE TABLE IF NOT EXISTS binary_uuid_examples (
    id INT PRIMARY KEY,
    label VARCHAR(100) NOT NULL,
    expected_uuid CHAR(36) NULL,
    standard_binary BINARY(16) NULL,
    standard_varbinary VARBINARY(16) NULL,
    swapped_binary BINARY(16) NULL,
    swapped_varbinary VARBINARY(16) NULL,
    expected_standard_hex CHAR(32) NULL,
    expected_swapped_hex CHAR(32) NULL
);

INSERT IGNORE INTO binary_uuid_examples
    (id, label, expected_uuid, standard_binary, standard_varbinary,
     swapped_binary, swapped_varbinary, expected_standard_hex, expected_swapped_hex)
VALUES
    (1, 'NULL UUID', NULL, NULL, NULL, NULL, NULL, NULL, NULL),
    (2, 'Nil UUID', '00000000-0000-0000-0000-000000000000', UUID_TO_BIN('00000000-0000-0000-0000-000000000000', 0), UUID_TO_BIN('00000000-0000-0000-0000-000000000000', 0), UUID_TO_BIN('00000000-0000-0000-0000-000000000000', 1), UUID_TO_BIN('00000000-0000-0000-0000-000000000000', 1), HEX(UUID_TO_BIN('00000000-0000-0000-0000-000000000000', 0)), HEX(UUID_TO_BIN('00000000-0000-0000-0000-000000000000', 1))),
    (3, 'All bits set', 'ffffffff-ffff-ffff-ffff-ffffffffffff', UUID_TO_BIN('ffffffff-ffff-ffff-ffff-ffffffffffff', 0), UUID_TO_BIN('ffffffff-ffff-ffff-ffff-ffffffffffff', 0), UUID_TO_BIN('ffffffff-ffff-ffff-ffff-ffffffffffff', 1), UUID_TO_BIN('ffffffff-ffff-ffff-ffff-ffffffffffff', 1), HEX(UUID_TO_BIN('ffffffff-ffff-ffff-ffff-ffffffffffff', 0)), HEX(UUID_TO_BIN('ffffffff-ffff-ffff-ffff-ffffffffffff', 1))),
    (4, 'Version 1: swap visibly changes order', '6ccd780c-baba-1026-9564-5b8c656024db', UUID_TO_BIN('6ccd780c-baba-1026-9564-5b8c656024db', 0), UUID_TO_BIN('6ccd780c-baba-1026-9564-5b8c656024db', 0), UUID_TO_BIN('6ccd780c-baba-1026-9564-5b8c656024db', 1), UUID_TO_BIN('6ccd780c-baba-1026-9564-5b8c656024db', 1), HEX(UUID_TO_BIN('6ccd780c-baba-1026-9564-5b8c656024db', 0)), HEX(UUID_TO_BIN('6ccd780c-baba-1026-9564-5b8c656024db', 1))),
    (5, 'Version 4', '550e8400-e29b-41d4-a716-446655440000', UUID_TO_BIN('550e8400-e29b-41d4-a716-446655440000', 0), UUID_TO_BIN('550e8400-e29b-41d4-a716-446655440000', 0), UUID_TO_BIN('550e8400-e29b-41d4-a716-446655440000', 1), UUID_TO_BIN('550e8400-e29b-41d4-a716-446655440000', 1), HEX(UUID_TO_BIN('550e8400-e29b-41d4-a716-446655440000', 0)), HEX(UUID_TO_BIN('550e8400-e29b-41d4-a716-446655440000', 1))),
    (6, 'Version 7', '01890f3e-7b5a-7cc2-98c4-dc0c0c07398f', UUID_TO_BIN('01890f3e-7b5a-7cc2-98c4-dc0c0c07398f', 0), UUID_TO_BIN('01890f3e-7b5a-7cc2-98c4-dc0c0c07398f', 0), UUID_TO_BIN('01890f3e-7b5a-7cc2-98c4-dc0c0c07398f', 1), UUID_TO_BIN('01890f3e-7b5a-7cc2-98c4-dc0c0c07398f', 1), HEX(UUID_TO_BIN('01890f3e-7b5a-7cc2-98c4-dc0c0c07398f', 0)), HEX(UUID_TO_BIN('01890f3e-7b5a-7cc2-98c4-dc0c0c07398f', 1)));

CREATE TABLE IF NOT EXISTS binary_edge_examples (
    id INT PRIMARY KEY,
    label VARCHAR(100) NOT NULL,
    payload BLOB NULL,
    expected_byte_length INT NULL,
    expected_hex TEXT NULL,
    text_control VARCHAR(200) NOT NULL
);

-- BLOB preserves zero length and arbitrary lengths. BINARY(16) would pad short
-- values, so it cannot accurately represent the empty/15/17-byte cases.
INSERT IGNORE INTO binary_edge_examples
    (id, label, payload, expected_byte_length, expected_hex, text_control)
VALUES
    (1, 'SQL NULL', NULL, NULL, NULL, 'TEXT control stays text: 550e8400-e29b-41d4-a716-446655440000'),
    (2, 'Empty bytes, distinct from NULL', X'', 0, '', 'TEXT control stays text: 550e8400-e29b-41d4-a716-446655440000'),
    (3, 'Single zero byte', X'00', 1, '00', 'TEXT control stays text: 550e8400-e29b-41d4-a716-446655440000'),
    (4, 'Single FF byte', X'FF', 1, 'FF', 'TEXT control stays text: 550e8400-e29b-41d4-a716-446655440000'),
    (5, 'Non UTF-8 and embedded zero', X'0080FFC32800', 6, '0080FFC32800', 'TEXT control stays text: 550e8400-e29b-41d4-a716-446655440000'),
    (6, '15 bytes: invalid UUID length', UNHEX(REPEAT('11', 15)), 15, REPEAT('11', 15), 'TEXT control stays text: 550e8400-e29b-41d4-a716-446655440000'),
    (7, '17 bytes: invalid UUID length', UNHEX(REPEAT('22', 17)), 17, REPEAT('22', 17), 'TEXT control stays text: 550e8400-e29b-41d4-a716-446655440000'),
    (8, 'JSON-looking binary: preserve bytes', X'7B226F6B223A747275657D', 11, '7B226F6B223A747275657D', 'TEXT control stays text: 550e8400-e29b-41d4-a716-446655440000'),
    (9, 'Every possible byte: 00 through FF', UNHEX('000102030405060708090A0B0C0D0E0F101112131415161718191A1B1C1D1E1F202122232425262728292A2B2C2D2E2F303132333435363738393A3B3C3D3E3F404142434445464748494A4B4C4D4E4F505152535455565758595A5B5C5D5E5F606162636465666768696A6B6C6D6E6F707172737475767778797A7B7C7D7E7F808182838485868788898A8B8C8D8E8F909192939495969798999A9B9C9D9E9FA0A1A2A3A4A5A6A7A8A9AAABACADAEAFB0B1B2B3B4B5B6B7B8B9BABBBCBDBEBFC0C1C2C3C4C5C6C7C8C9CACBCCCDCECFD0D1D2D3D4D5D6D7D8D9DADBDCDDDEDFE0E1E2E3E4E5E6E7E8E9EAEBECEDEEEFF0F1F2F3F4F5F6F7F8F9FAFBFCFDFEFF'), 256, '000102030405060708090A0B0C0D0E0F101112131415161718191A1B1C1D1E1F202122232425262728292A2B2C2D2E2F303132333435363738393A3B3C3D3E3F404142434445464748494A4B4C4D4E4F505152535455565758595A5B5C5D5E5F606162636465666768696A6B6C6D6E6F707172737475767778797A7B7C7D7E7F808182838485868788898A8B8C8D8E8F909192939495969798999A9B9C9D9E9FA0A1A2A3A4A5A6A7A8A9AAABACADAEAFB0B1B2B3B4B5B6B7B8B9BABBBCBDBEBFC0C1C2C3C4C5C6C7C8C9CACBCCCDCECFD0D1D2D3D4D5D6D7D8D9DADBDCDDDEDFE0E1E2E3E4E5E6E7E8E9EAEBECEDEEEFF0F1F2F3F4F5F6F7F8F9FAFBFCFDFEFF', 'TEXT control stays text: 550e8400-e29b-41d4-a716-446655440000'),
    (10, '4096 bytes: test truncated display and full copy', UNHEX(REPEAT('00FF80A5', 1024)), 4096, REPEAT('00FF80A5', 1024), 'TEXT control stays text: 550e8400-e29b-41d4-a716-446655440000'),
    (11, 'UTF-8 binary with whitespace', X'48656C6C6F20090A', 8, '48656C6C6F20090A', 'TEXT control stays text: 550e8400-e29b-41d4-a716-446655440000');

CREATE TABLE IF NOT EXISTS binary_primary_key_examples (
    binary_id BINARY(16) PRIMARY KEY,
    label VARCHAR(100) NOT NULL,
    expected_uuid CHAR(36) NOT NULL,
    expected_original_hex CHAR(32) NOT NULL,
    editable_note VARCHAR(200) NOT NULL
);

-- View binary_id as standard UUID. Edit editable_note first, then binary_id;
-- commit and reload to check updates use the ORIGINAL binary primary key.
-- Changing a key to an existing key should fail without losing staged changes.
INSERT IGNORE INTO binary_primary_key_examples
    (binary_id, label, expected_uuid, expected_original_hex, editable_note)
VALUES
    (UUID_TO_BIN('6ccd780c-baba-1026-9564-5b8c656024db', 0), 'Version 1: swap visibly changes order', '6ccd780c-baba-1026-9564-5b8c656024db', HEX(UUID_TO_BIN('6ccd780c-baba-1026-9564-5b8c656024db', 0)), 'Edit this note, then edit the binary primary key'),
    (UUID_TO_BIN('550e8400-e29b-41d4-a716-446655440000', 0), 'Version 4', '550e8400-e29b-41d4-a716-446655440000', HEX(UUID_TO_BIN('550e8400-e29b-41d4-a716-446655440000', 0)), 'Edit this note, then edit the binary primary key'),
    (UUID_TO_BIN('01890f3e-7b5a-7cc2-98c4-dc0c0c07398f', 0), 'Version 7', '01890f3e-7b5a-7cc2-98c4-dc0c0c07398f', HEX(UUID_TO_BIN('01890f3e-7b5a-7cc2-98c4-dc0c0c07398f', 0)), 'Edit this note, then edit the binary primary key');

-- View guidance:
-- binary_uuid_examples: standard_* => UUID; swapped_* => UUID (MySQL swapped).
-- Their text/hex references should match exactly before editing. Try changing
-- one UUID, committing, reloading, then switching to hex to verify stored bytes.
-- binary_edge_examples: start with hex; test binary digits and UUID length
-- errors. Empty hex means empty bytes; use the explicit NULL action for SQL NULL.
-- Try malformed hex (odd digits, nonhex, +f); no invalid value should be staged.
-- Copy the 4096-byte cell and paste into a binary cell: display may be truncated,
-- clipboard data must remain complete. Avoid using reference columns as targets.
-- Binary/UUID view selection must leave text_control as ordinary text.
-- Tables initially contain 6 UUID rows, 11 edge rows and 3 binary-key rows.


-- Readable BLOBs default to Text, including JSON. NULL does not affect detection.
-- Check the table grid and `SELECT * FROM binary_json_examples`, then open row
-- details: JSON should be pretty-printed. Copy/edit the payload and switch to
-- Hex to confirm whitespace, Unicode and large numbers retain their bytes.
CREATE TABLE IF NOT EXISTS binary_json_examples (
    id INT PRIMARY KEY,
    payload BLOB NULL
);
INSERT IGNORE INTO binary_json_examples (id, payload) VALUES
    (1, NULL),
    (2, _utf8mb4'{"ok":true,"nested":{"roles":["admin"]}}'),
    (3, _utf8mb4' {"label":"café","id":900719925474099312345} '),
    (4, _utf8mb4'[1,true,null,"hello"]'),
    (5, _utf8mb4'null'),
    (6, X'');
-- The mixed binary_edge_examples column defaults to Hex. Choose Text from its
-- cell menu or row-details format menu to read row 8 as JSON and row 11 as text;
-- non-text bytes still display and edit as Hex.
