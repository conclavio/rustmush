-- Offline first-staff bootstrap. Any account key, including a corrupt one,
-- prevents bootstrap. No creator can interleave the empty check and insert.
local cursor = '0'
repeat
    local page = redis.call('SCAN', cursor, 'MATCH', 'account:*', 'COUNT', 100)
    if #page[2] > 0 then return 0 end
    cursor = page[1]
until cursor == '0'
redis.call('JSON.SET', KEYS[1], '$', ARGV[1], 'NX')
return 1
