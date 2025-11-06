export default {
  calls: [
    {
      bank: {
        create_token: {
          token_name: "token_1",
          initial_balance: "20000",
          token_decimals: 12,
          supply_cap: "100000000000",
          mint_to_address: {
            Standard: "sov1lzkjgdaz08su3yevqu6ceywufl35se9f33kztu5cu2spja5hyyf",
          },
          admins: [
            {
              Standard:
                "sov1lzkjgdaz08su3yevqu6ceywufl35se9f33kztu5cu2spja5hyyf",
            },
          ],
        },
      },
    },
    {
      bank: {
        create_token: {
          token_name: "token_1",
          initial_balance: 20000,
          token_decimals: 12,
          supply_cap: "100000000000",
          mint_to_address: {
            Standard: "sov1lzkjgdaz08su3yevqu6ceywufl35se9f33kztu5cu2spja5hyyf",
          },
          admins: [],
        },
      },
    },
  ],
};
