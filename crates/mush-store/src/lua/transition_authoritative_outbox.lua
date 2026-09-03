-- Exact CAS transition for a durable-outbox record.
--
-- KEYS: item record, pending index.
-- ARGV: exact prior doc, exact next doc, item id, next-is-pending (0/1).

local function key_type(key)
  local result = redis.call('TYPE', key)
  if type(result) == 'table' then
    return result['ok']
  end
  return result
end

local pending_type = key_type(KEYS[2])
if pending_type ~= 'none' and pending_type ~= 'set' then
  return 'CORRUPT_OUTBOX_PENDING'
end

local current = redis.call('GET', KEYS[1])
if not current then
  return 'OUTBOX_MISSING'
end
if current == ARGV[2] then
  return 'RESUMED'
end
if current ~= ARGV[1] then
  return 'OUTBOX_CONFLICT'
end
if ARGV[4] ~= '0' and ARGV[4] ~= '1' then
  return 'CORRUPT_ARGUMENTS'
end

redis.call('SET', KEYS[1], ARGV[2])
if ARGV[4] == '1' then
  redis.call('SADD', KEYS[2], ARGV[3])
else
  redis.call('SREM', KEYS[2], ARGV[3])
end
return 'UPDATED'
