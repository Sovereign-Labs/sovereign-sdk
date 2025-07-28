import argparse
import sys
import pandas as pd
import os

def transpose_constants(df: pd.DataFrame) -> pd.DataFrame:
    print("Unique state roots", len(df['pre_state_root'].unique()))

    const_df_dic = {}
    unique_const_values = df['constant'].unique()
    for const in unique_const_values:
        constant_colum = df[df['constant'] == const].rename(columns={'num_invocations': const})
        constant_colum.drop(['constant', 'name'], axis=1, inplace=True)
        const_df_dic[const] = constant_colum


    # We start merging from the fixed gas to charge per signature verification constant
    merged_df = const_df_dic.pop('DEFAULT_FIXED_GAS_TO_CHARGE_PER_SIGNATURE_VERIFICATION')

    # Iteratively merge with the rest of the DataFrames in the dict
    for df in const_df_dic.values():
        merged_df = pd.merge(merged_df, df, on='pre_state_root', how='outer', validate='one_to_one')

    # move pre_state_root column to the front
    col_to_move = 'pre_state_root'
    merged_df = merged_df[[col_to_move] + [col for col in merged_df.columns if col != col_to_move]]
    return merged_df

root_dir = "../data"

def process_files():
    global_transposed_df = pd.DataFrame()
    global_zkvm_df = pd.DataFrame()
    for dirpath, dirnames, filenames in os.walk(root_dir):
        for filename in filenames:
            # Transpose the constants files and merges them together.
            if filename == "constants_output.csv":
                file_path = os.path.join(dirpath, filename)
                df = pd.read_csv(file_path)
                transposed_df = transpose_constants(df)
                global_transposed_df = pd.concat([global_transposed_df, transposed_df], ignore_index=True)

                target_file_path = os.path.join(dirpath, "transposed_constants_output.csv")
                transposed_df.to_csv(target_file_path, index=False)

            # Merges zk_vm files.
            if filename == "zk_vm.csv":
                file_path = os.path.join(dirpath, filename)
                df = pd.read_csv(file_path)
                global_zkvm_df = pd.concat([global_zkvm_df, df], ignore_index=True)

    global_transposed_df.to_csv("../data/global_constants_output.csv", index=False)
    global_zkvm_df.to_csv("../data/global_zk_vm_output.csv", index=False)
    
if __name__ == "__main__":
    process_files()