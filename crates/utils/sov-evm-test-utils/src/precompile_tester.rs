use alloy_sol_types::sol;

// Generated from the following Solidity code
// // SPDX-License-Identifier: MIT
//  pragma solidity ^0.8.0;
//
//  contract PrecompileChecker {
//      /// @notice Calls a precompile and asserts the return value matches expected
//      /// @param precompile The address of the precompile to call
//      /// @param input The input data to pass to the precompile
//      /// @param expectedOutput The expected return data from the precompile
//      /// @dev Reverts if the call fails or the return value doesn't match
//      function assertPrecompileResult(
//          address precompile,
//          bytes calldata input,
//          bytes calldata expectedOutput
//      ) external view {
//          // Call the precompile
//          (bool success, bytes memory result) = precompile.staticcall(input);
//
//          // Revert if the call failed
//          require(success, "Precompile call failed");
//
//          // Revert if the return data length doesn't match
//          require(result.length == expectedOutput.length, "Return data length mismatch");
//
//          // Compare the return data byte by byte
//          for (uint256 i = 0; i < result.length; i++) {
//              require(result[i] == expectedOutput[i], "Return data mismatch");
//          }
//      }
//
//      /// @notice Calls a precompile and asserts it fails (for testing disabled precompiles)
//      /// @param precompile The address of the precompile to call
//      /// @param input The input data to pass to the precompile
//      /// @dev Reverts if the call succeeds (we expect it to fail)
//      function assertPrecompileFails(
//          address precompile,
//          bytes calldata input
//      ) external view {
//          (bool success, ) = precompile.staticcall(input);
//          require(!success, "Expected precompile call to fail but it succeeded");
//      }
//
//      /// @notice Calls a precompile and returns the raw result (for debugging)
//      /// @param precompile The address of the precompile to call
//      /// @param input The input data to pass to the precompile
//      /// @return success Whether the call succeeded
//      /// @return result The return data from the precompile
//      function callPrecompile(
//          address precompile,
//          bytes calldata input
//      ) external view returns (bool success, bytes memory result) {
//          (success, result) = precompile.staticcall(input);
//      }
//  }
sol!(
    #[sol(
        rpc,
        all_derives = true,
        bytecode = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/contracts/artifacts/", "PrecompileTester.bin")))]
    PrecompileTester,
    concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/contracts/artifacts/",
        "PrecompileTester.abi"
    )
);
