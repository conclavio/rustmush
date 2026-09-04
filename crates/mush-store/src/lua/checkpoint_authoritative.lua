-- Atomically release one finalized full intent after preserving its compact
-- canonical receipt. The old receipt bytes are supplied back as a CAS guard,
-- so a concurrent or externally clobbered witness can never be overwritten.
--
-- KEYS: intent, receipt, idempotency binding, pending set, retained count,
-- retained bytes, retained generation, exact parent outbox manifest, then
-- every exact terminal outbox item.
-- ARGV: exact intent doc, exact prior receipt doc, compact checkpoint receipt,
-- exact binding doc, token, exact intent bytes, outbox count, exact parent
-- manifest, then each exact
-- terminal outbox document.

local function key_type(key)
  local result = redis.call('TYPE', key)
  if type(result) == 'table' then
    return result['ok']
  end
  return result
end


local function canonical_decimal(value)
  return value and string.match(value, '^%d+$') and
    (value == '0' or string.sub(value, 1, 1) ~= '0')
end

local pending_type = key_type(KEYS[4])
if pending_type ~= 'none' and pending_type ~= 'set' then
  return 'CORRUPT_PENDING'
end
local parent_manifest_type = key_type(KEYS[8])
if parent_manifest_type ~= 'none' and parent_manifest_type ~= 'string' then
  return 'OUTBOX_CONFLICT'
end

local intent = redis.call('GET', KEYS[1])
if not intent then
  if redis.call('GET', KEYS[2]) == ARGV[3] and redis.call('GET', KEYS[3]) == ARGV[4] then
    redis.call('SREM', KEYS[4], ARGV[5])
    return 'RESUMED'
  end
  if redis.call('EXISTS', KEYS[2]) ~= 0 then
    return 'TOKEN_CONFLICT'
  end
  return 'MISSING_INTENT'
end
if intent ~= ARGV[1] then
  return 'TOKEN_CONFLICT'
end
if redis.call('GET', KEYS[2]) ~= ARGV[2] then
  if redis.call('EXISTS', KEYS[2]) == 0 then
    return 'NOT_FINALIZED'
  end
  return 'TOKEN_CONFLICT'
end
if redis.call('GET', KEYS[3]) ~= ARGV[4] then
  return 'CORRUPT_BINDING'
end
if redis.call('SISMEMBER', KEYS[4], ARGV[5]) ~= 0 then
  return 'NOT_FINALIZED'
end

local count_doc = redis.call('GET', KEYS[5])
local bytes_doc = redis.call('GET', KEYS[6])
local intent_bytes = tonumber(ARGV[6])
local retained_count = tonumber(count_doc or '')
local retained_bytes = tonumber(bytes_doc or '')
if not retained_count or not retained_bytes or not intent_bytes or
   retained_count < 1 or retained_bytes < intent_bytes then
  return 'CORRUPT_COUNTER'
end

local outbox_count = tonumber(ARGV[7])
if not outbox_count or outbox_count < 0 or not ARGV[8] or
   outbox_count ~= (#KEYS - 8) then
  return 'CORRUPT_ARGUMENTS'
end
local parent_manifest = redis.call('GET', KEYS[8])
if parent_manifest and parent_manifest ~= ARGV[8] then
  return 'OUTBOX_CONFLICT'
end
for index = 1, outbox_count do
  if redis.call('GET', KEYS[8 + index]) ~= ARGV[8 + index] then
    return 'OUTBOX_PENDING'
  end
end

local generation_type = key_type(KEYS[7])
if generation_type ~= 'none' and generation_type ~= 'string' then
  return 'CORRUPT_GENERATION'
end
local generation = redis.call('GET', KEYS[7]) or '0'
if not canonical_decimal(generation) or
   (string.len(generation) == 19 and generation > '9223372036854775806') or
   string.len(generation) > 19 then
  return 'CORRUPT_GENERATION'
end

redis.call('INCR', KEYS[7])
redis.call('SET', KEYS[8], ARGV[8])
redis.call('SET', KEYS[2], ARGV[3])
redis.call('DEL', KEYS[1])
redis.call('SET', KEYS[5], tostring(retained_count - 1))
redis.call('SET', KEYS[6], tostring(retained_bytes - intent_bytes))
return 'CHECKPOINTED'
