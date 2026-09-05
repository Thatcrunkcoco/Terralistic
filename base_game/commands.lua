function command_give(arguments, executor)
    if #arguments < 1 then
        return "Command 'give' requires at least one argument."
    end
    
    if #arguments > 3 then
        return "Command 'give' takes at most three arguments."
    end
    
    player = executor
    item_name = arguments[1]
    
    item = terralistic_get_item_id_by_name(item_name)
    if item == nil then
        return "Item '" .. item_name .. "' not found."
    end
    
    amount = 1
    if #arguments >= 2 then
        amount = tonumber(arguments[2])
        if amount == nil then
            return "Argument 2 must be a number."
        end
    end
    
    if #arguments >= 3 then
        player = terralistic_get_player_by_name(arguments[3])
        if player == nil then
            return "Player '" .. arguments[3] .. "' not found."
        end
    end
    
    if player == nil then
        return "No player specified."
    end
    
    terralistic_give_item(player, item, amount)
    
    return "Gave item"
end

function describe_command_give() 
    return 
[[Gives the player an item.
Usage: give <item> [amount] [player] - amount defaults to 1, player defaults to the executor.]]
end

function command_stop(arguments, executor)
    if #arguments ~= 0 then
        return "Command 'stop' does not take any arguments."
    end
    
    terralistic_stop_server()
    return "Stopping the server..."
end

function describe_command_stop()
    return "Stops the server."
end

function command_pos(arguments, executor)
    if #arguments ~= 0 then
        return "Command 'pos' does not take any arguments."
    end
    if executor == nil then
        return "Command 'pos' must be run by a player."
    end
    x, y = terralistic_get_player_position(executor)
    return "You are at x=" .. math.floor(x) .. ", y=" .. math.floor(y)
end

function describe_command_pos()
    return [[Reports your position.
Usage: pos]]
end

function command_tp(arguments, executor)
    if #arguments ~= 2 and #arguments ~= 3 then
        return "Usage: tp <x> <y> [player]"
    end

    target = executor
    offset = 0
    if #arguments == 3 then
        target = terralistic_get_player_by_name(arguments[1])
        offset = 1
        if target == nil then
            return "Player '" .. arguments[1] .. "' not found."
        end
    end
    if target == nil then
        return "No player specified."
    end

    x = tonumber(arguments[1 + offset])
    y = tonumber(arguments[2 + offset])
    if x == nil or y == nil then
        return "Arguments must be numbers."
    end

    terralistic_set_player_position(target, x, y)
    return "Teleported to x=" .. math.floor(x) .. ", y=" .. math.floor(y)
end

function describe_command_tp()
    return [[Teleports a player to coordinates.
Usage: tp <x> <y> [player] - player defaults to the executor.]]
end

function command_heal(arguments, executor)
    if #arguments > 2 then
        return "Usage: heal [amount] [player]"
    end

    target = executor
    amount = 100
    offset = 0
    if #arguments >= 1 and tonumber(arguments[1]) ~= nil then
        amount = tonumber(arguments[1])
        health_arg = true
    else
        health_arg = false
    end
    if #arguments == 2 then
        target = terralistic_get_player_by_name(arguments[2])
        if target == nil then
            return "Player '" .. arguments[2] .. "' not found."
        end
    elseif #arguments == 1 and not health_arg then
        target = terralistic_get_player_by_name(arguments[1])
        if target == nil then
            return "Player '" .. arguments[1] .. "' not found."
        end
    end
    if target == nil then
        return "No player specified."
    end

    terralistic_set_player_health(target, amount)
    return "Set health to " .. amount
end

function describe_command_heal()
    return [[Sets a player's health.
Usage: heal [amount] [player] - amount defaults to 100, player defaults to the executor.]]
end

function command_test(arguments, executor)
    return [[Flat test world: you spawn near the middle. Walk WEST (toward x=0) to find the labeled sections in order. All structures are jumpable - no digging needed:
pillar - a short stone block stack
pit - shallow hole you can jump out of
mount - low stone steps
ores - columns of copper/iron/tin (embedded, mine to collect)
platform - low wooden steps
torches - torch line (light test)
house - small low wood house
wall - a short stone wall to step over]]
end

function describe_command_test()
    return "Lists the test sections and where to find them."
end
function command_setblock(arguments, executor)
    if #arguments ~= 3 then
        return "Usage: setblock <block name> <x> <y> - coordinates in block units"
    end

    x = tonumber(arguments[2])
    y = tonumber(arguments[3])
    if x == nil or y == nil then
        return "Arguments 2 and 3 must be numbers."
    end

    block_id = terralistic_get_block_id_by_name(arguments[1])
    if block_id == nil then
        return "Block '" .. arguments[1] .. "' not found."
    end

    terralistic_set_block(x, y, block_id)
    return "Set block"
end

function describe_command_setblock()
    return [[Sets a world block directly.
Usage: setblock <block name> <x> <y> - coordinates in block units.
Test-harness helper for scripted visual scenes.]]
end
