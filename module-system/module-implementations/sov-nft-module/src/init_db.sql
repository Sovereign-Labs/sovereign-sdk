-- Drop existing tables if they exist
DROP TABLE IF EXISTS top_owners CASCADE;
DROP TABLE IF EXISTS nfts CASCADE;
DROP TABLE IF EXISTS collections CASCADE;

-- Create collection table
CREATE TABLE collections
(
    collection_address TEXT PRIMARY KEY,
    collection_name    TEXT    NOT NULL,
    creator_address    TEXT    NOT NULL,
    frozen             BOOLEAN NOT NULL,
    metadata_url       TEXT,
    supply             BIGINT  NOT NULL
);

-- Create index on creator_address to quickly find collections by a creator
CREATE INDEX idx_creator ON collections (creator_address);

-- Create nft table
CREATE TABLE nfts
(
    collection_address TEXT    NOT NULL,
    nft_id             BIGINT  NOT NULL,
    metadata_url       TEXT,
    owner              TEXT    NOT NULL,
    frozen             BOOLEAN NOT NULL,
    PRIMARY KEY (collection_address, nft_id)
);

-- Create index on owner to quickly find NFTs owned by a particular address
CREATE INDEX idx_nft_owner ON nfts (owner);

-- Create index on collection_address to quickly find NFTs belonging to a particular collection
CREATE INDEX idx_nft_collection ON nfts (collection_address);

-- Create top_owners table
CREATE TABLE top_owners
(
    owner              TEXT   NOT NULL,
    collection_address TEXT   NOT NULL,
    count              BIGINT NOT NULL,
    PRIMARY KEY (owner, collection_address)
);

-- Create index on collection_address to quickly find top owners in a particular collection
CREATE INDEX idx_top_owners_collection ON top_owners (collection_address);

-- Create index on count to quickly find top owners by count (optional)
CREATE INDEX idx_top_owners_count ON top_owners (count);
