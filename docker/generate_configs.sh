MOUNT_FOLDER=keyring-test
NODE_1_KEY_FILE=bridge_1_key.txt
NODE_2_KEY_FILE=bridge_2_key.txt

count=0; while [[ ! -f "$MOUNT_FOLDER/$NODE_1_KEY_FILE" && $count -lt 300 ]]; do sleep 1; ((count++)); done

NODE_1_KEY="$(cat "$MOUNT_FOLDER/$NODE_1_KEY_FILE" | egrep -v '^$|^WARNING|^\*\*DO NOT')";
sed "s/^celestia_rpc_auth_token = .*/celestia_rpc_auth_token = \"$NODE_1_KEY\"/g" template.toml | \
 sed "s/^path = .*/path = \"demo_data_1\"/g" \
 > config_1.toml;

count=0; while [[ ! -f "$MOUNT_FOLDER/$NODE_2_KEY_FILE" && $count -lt 300 ]]; do sleep 1; ((count++)); done

NODE_1_KEY="$(cat "$MOUNT_FOLDER/$NODE_2_KEY_FILE" | egrep -v '^$|^WARNING|^\*\*DO NOT')";
sed "s/^celestia_rpc_auth_token = .*/celestia_rpc_auth_token = \"$NODE_1_KEY\"/g" template.toml | \
 sed "s/^path = .*/path = \"demo_data_2\"/g" | \
 sed "s/^celestia_rpc_address = .*/celestia_rpc_address = \"http:\/\/127.0.0.1:46658\"/g" | \
 sed "s/^bind_port = .*/bind_port = 12346/g" \
 > config_2.toml;