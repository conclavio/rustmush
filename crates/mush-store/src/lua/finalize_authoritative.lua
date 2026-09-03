-- KEYS: intent, receipt, idempotency binding, pending set, outbox pending
-- set, then one key per normalized replay write, then one key per durable
-- outbox item.
-- ARGV: exact intent doc, exact receipt doc, exact binding doc, token,
-- replay-write count, outbox count, max pending outbox, then
-- (kind, exact post-image) replay pairs and (item id, exact record) pairs.

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
local outbox_pending_type = key_type(KEYS[5])
if outbox_pending_type ~= 'none' and outbox_pending_type ~= 'set' then
  return 'CORRUPT_OUTBOX_PENDING'
end

local intent = redis.call('GET', KEYS[1])
if not intent then
  return 'MISSING_INTENT'
end
if intent ~= ARGV[1] then
  return 'TOKEN_CONFLICT'
end
if redis.call('GET', KEYS[3]) ~= ARGV[3] then
  return 'CORRUPT_BINDING'
end

local receipt = redis.call('GET', KEYS[2])
if receipt then
  if receipt == ARGV[2] then
    redis.call('SREM', KEYS[4], ARGV[4])
    return 'FINALIZED'
  end
  return 'TOKEN_CONFLICT'
end

local write_count = tonumber(ARGV[5])
local outbox_count = tonumber(ARGV[6])
local max_pending_outbox = tonumber(ARGV[7])
if not write_count or write_count < 0 or not outbox_count or
   outbox_count < 0 or not max_pending_outbox or max_pending_outbox < 0 or
   (write_count + outbox_count) ~= (#KEYS - 5) then
  return 'CORRUPT_ARGUMENTS'
end

local outbox_base = 7 + write_count * 2
local new_outbox = 0
for index = 1, outbox_count do
  local item_id = ARGV[outbox_base + (index - 1) * 2 + 1]
  local item_doc = ARGV[outbox_base + (index - 1) * 2 + 2]
  local key = KEYS[5 + write_count + index]
  local existing = redis.call('GET', key)
  if existing and existing ~= item_doc then
    return 'OUTBOX_CONFLICT'
  end
  if not existing then
    new_outbox = new_outbox + 1
  end
  if not item_id or not item_doc then
    return 'CORRUPT_ARGUMENTS'
  end
end
local attempted_outbox = redis.call('SCARD', KEYS[5]) + new_outbox
if attempted_outbox > max_pending_outbox then
  return 'OUTBOX_CAPACITY:' .. tostring(attempted_outbox)
end

for index = 1, write_count do
  local kind = ARGV[6 + index * 2]
  local post_image = ARGV[7 + index * 2]
  local key = KEYS[5 + index]
  if kind == 'string_set' then
    local current = redis.call('GET', key)
    if current and (not tonumber(current) or tonumber(current) > tonumber(post_image)) then
      return 'ALLOCATOR_BEHIND'
    end
  end
end

for index = 1, write_count do
  local kind = ARGV[6 + index * 2]
  local post_image = ARGV[7 + index * 2]
  local key = KEYS[5 + index]
  if kind == 'json_set' then
    redis.call('JSON.SET', key, '$', post_image)
  elseif kind == 'string_set' then
    redis.call('SET', key, post_image)
  elseif kind == 'delete' then
    redis.call('DEL', key)
  else
    return 'CORRUPT_ARGUMENTS'
  end
end

for index = 1, write_count do
  local kind = ARGV[6 + index * 2]
  local post_image = ARGV[7 + index * 2]
  local key = KEYS[5 + index]
  if kind == 'json_set' then
    if redis.call('JSON.GET', key) ~= post_image then
      return 'READBACK_MISMATCH'
    end
  elseif kind == 'string_set' then
    if redis.call('GET', key) ~= post_image then
      return 'READBACK_MISMATCH'
    end
  elseif kind == 'delete' then
    if redis.call('EXISTS', key) ~= 0 then
      return 'READBACK_MISMATCH'
    end
  end
end

for index = 1, outbox_count do
  local item_id = ARGV[outbox_base + (index - 1) * 2 + 1]
  local item_doc = ARGV[outbox_base + (index - 1) * 2 + 2]
  local key = KEYS[5 + write_count + index]
  if redis.call('EXISTS', key) == 0 then
    redis.call('SET', key, item_doc)
  end
  redis.call('SADD', KEYS[5], item_id)
end

redis.call('SET', KEYS[2], ARGV[2])
redis.call('SREM', KEYS[4], ARGV[4])
return 'FINALIZED'
