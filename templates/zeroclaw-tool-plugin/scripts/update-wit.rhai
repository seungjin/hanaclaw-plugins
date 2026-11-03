// cargo-generate post hook: refresh the bundled WIT contract.
// This runs after the project is generated, in the project directory.
system::command("bash", ["scripts/update-wit.sh"]);
