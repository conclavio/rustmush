-- Atomically release one finalized full intent after preserving its compact
-- canonical receipt. The old receipt bytes are supplied back as a CAS guard,
-- so a concurrent or externally clobbered witness can never be overwritten.
--
-- KEYS: intent, receipt, idempotency binding, pending set, retained count,
-- retained bytes.
-- ARGV: exact intent doc, exact prior receipt doc, compact checkpoint receipt,
-- exact binding doc, token, exact intent bytes.

local function key_type(key)
  local result = redis.call('TYPE', key)
  if type(result) == 'table' then
    return result['ok']
  end
  return result
end

local pending_type = key_type(KEYS[4])
if pending_type ~= 'none' and pending_type ~= 'set' then
  return 'CORRUPT_PENDING'
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

redis.call('SET', KEYS[2], ARGV[3])
redis.call('DEL', KEYS[1])
redis.call('SET', KEYS[5], tostring(retained_count - 1))
redis.call('SET', KEYS[6], tostring(retained_bytes - intent_bytes))
return 'CHECKPOINTED'
