-- KEYS: intent, receipt, idempotency binding, pending set, then one key per
-- normalized replay write.
-- ARGV: exact intent doc, exact receipt doc, exact binding doc, token,
-- replay-write count, then (kind, exact post-image) pairs.

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
if not write_count or write_count < 0 or write_count ~= (#KEYS - 4) then
  return 'CORRUPT_ARGUMENTS'
end

for index = 1, write_count do
  local kind = ARGV[4 + index * 2]
  local post_image = ARGV[5 + index * 2]
  local key = KEYS[4 + index]
  if kind == 'string_set' then
    local current = redis.call('GET', key)
    if current and (not tonumber(current) or tonumber(current) > tonumber(post_image)) then
      return 'ALLOCATOR_BEHIND'
    end
  end
end

for index = 1, write_count do
  local kind = ARGV[4 + index * 2]
  local post_image = ARGV[5 + index * 2]
  local key = KEYS[4 + index]
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
  local kind = ARGV[4 + index * 2]
  local post_image = ARGV[5 + index * 2]
  local key = KEYS[4 + index]
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

redis.call('SET', KEYS[2], ARGV[2])
redis.call('SREM', KEYS[4], ARGV[4])
return 'FINALIZED'
