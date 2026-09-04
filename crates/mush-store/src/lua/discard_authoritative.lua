-- Atomically release one never-finalized full intent after its complete
-- bundle has entered the independent quarantine spool. The idempotency
-- binding and a compact exact quarantine receipt remain.
--
-- KEYS: intent, finalized receipt, idempotency binding, pending set,
-- retained count, retained bytes, retained generation, quarantine receipt.
-- ARGV: exact intent doc, exact binding doc, exact quarantine receipt, token,
-- exact intent bytes.

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

local intent = redis.call('GET', KEYS[1])
if not intent then
  if redis.call('GET', KEYS[8]) == ARGV[3] and
     redis.call('GET', KEYS[3]) == ARGV[2] then
    redis.call('SREM', KEYS[4], ARGV[4])
    return 'RESUMED'
  end
  if redis.call('EXISTS', KEYS[2]) ~= 0 then
    return 'ALREADY_FINALIZED'
  end
  if redis.call('EXISTS', KEYS[8]) ~= 0 then
    return 'TOKEN_CONFLICT'
  end
  return 'MISSING_INTENT'
end
if intent ~= ARGV[1] then
  return 'TOKEN_CONFLICT'
end
if redis.call('EXISTS', KEYS[2]) ~= 0 then
  return 'ALREADY_FINALIZED'
end
if redis.call('GET', KEYS[3]) ~= ARGV[2] then
  return 'CORRUPT_BINDING'
end
if redis.call('EXISTS', KEYS[8]) ~= 0 then
  return 'TOKEN_CONFLICT'
end

local count_doc = redis.call('GET', KEYS[5])
local bytes_doc = redis.call('GET', KEYS[6])
local intent_bytes = tonumber(ARGV[5])
local retained_count = tonumber(count_doc or '')
local retained_bytes = tonumber(bytes_doc or '')
if not retained_count or not retained_bytes or not intent_bytes or
   retained_count < 1 or retained_bytes < intent_bytes then
  return 'CORRUPT_COUNTER'
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

redis.call('SET', KEYS[8], ARGV[3])
redis.call('INCR', KEYS[7])
redis.call('DEL', KEYS[1])
redis.call('SREM', KEYS[4], ARGV[4])
redis.call('SET', KEYS[5], tostring(retained_count - 1))
redis.call('SET', KEYS[6], tostring(retained_bytes - intent_bytes))
return 'DISCARDED'
