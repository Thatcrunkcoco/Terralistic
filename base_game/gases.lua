--[[
Substance type declarations for the base game.

Every substance — gas *or* liquid — is a single registered "gas type" on the
same shared substance cell layer. A substance declares a unique name and a
density; density drives vertical layering during flow. The layer, registry, and
flow simulation treat gases and liquids identically: a liquid is just a
substance heavy enough to sink below the air and pool on solid floors, rather
than a wholly separate simulation system.

Gases are lighter and spread/rise; liquids are much denser and settle low.
]]--

gases = {}

function register_gases()
    terralistic_print("registering substances (gases + liquids)...")

    -- Liquids are far denser than any gas, so they sink to the bottom of a
    -- shared pocket and pool on solid floors while lighter gases stay above.
    gases.water = terralistic_register_gas_type("water", 1000.0)
    -- A second liquid for contrast; heavier than water so it settles beneath it.
    gases.magma = terralistic_register_gas_type("magma", 3200.0)

    -- Default breathable atmosphere.
    gases.air = terralistic_register_gas_type("air", 1.2)

    -- CO2 is heavier than air, so it pools low in sealed bases.
    gases.co2 = terralistic_register_gas_type("co2", 1.98)

    -- Oxygen breathes; declared separately for future breathability logic.
    gases.oxygen = terralistic_register_gas_type("oxygen", 1.43)

    -- Hydrogen is much lighter than air and rises.
    gases.hydrogen = terralistic_register_gas_type("hydrogen", 0.09)
end
