import urllib.request
import json
import sys

# Requires Python 3.11+ for the built-in TOML parser
try:
    import tomllib
except ImportError:
    print("Error: Python 3.11 or newer is required to use the built-in 'tomllib' module.")
    sys.exit(1)

def get_latest_version(crate_name):
    url = f"https://crates.io/api/v1/crates/{crate_name}"
    # Crates.io requires a descriptive User-Agent
    req = urllib.request.Request(url, headers={'User-Agent': 'VersionCheckerScript/1.0'})
    try:
        with urllib.request.urlopen(req) as response:
            data = json.loads(response.read().decode())
            return data['crate']['max_stable_version']
    except Exception as e:
        return f"Error: {e}"

def main():
    # Load and parse the Cargo.toml file from the current directory
    try:
        with open("Cargo.toml", "rb") as f:
            cargo_data = tomllib.load(f)
    except FileNotFoundError:
        print("Error: 'Cargo.toml' not found in the current directory.")
        sys.exit(1)
    
    dependencies = cargo_data.get("dependencies", {})
    if not dependencies:
        print("No dependencies found in Cargo.toml.")
        return

    print(f"{'Crate':<20} | {'Current':<10} | {'Latest'}")
    print("-" * 45)
    
    for crate, requirement in dependencies.items():
        # Handle simple string versions vs inline tables (e.g., { version = "X", features = [...] })
        if isinstance(requirement, dict):
            current_ver = requirement.get("version", "unknown")
        else:
            current_ver = str(requirement)
        
        latest = get_latest_version(crate)
        print(f"{crate:<20} | {current_ver:<10} | {latest}")

if __name__ == "__main__":
    main()