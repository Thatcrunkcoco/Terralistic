--[[
Gas type declarations for the base game.

Each gas declares a unique name and a density. Density drives vertical layering
during flow: heavier gases sink, lighter gases rise. The "air" gas is the default
atmosphere the world is seeded with.
]]--

gases = {}

function register_gases()
    terralistic_print("registering gases...")

    -- Default breathable atmosphere.
    gases.air = terralistic_register_gas_type("air", 1.2)

    -- CO2 is heavier than air, so it pools low in sealed bases.
    gases.co2 = terralistic_register_gas_type("co2", 1.98)

    -- Oxygen breathes; declared separately for future breathability logic.
    gases.oxygen = terralistic_register_gas_type("oxygen", 1.43)

    -- Hydrogen is much lighter than air and rises.
    gases.hydrogen = terralistic_register_gas_type("hydrogen", 0.09)
end
