-- Disposable qualification fixtures, NOT the production forge schema.
DEFINE TABLE gitr_ref SCHEMAFULL;
DEFINE FIELD oid ON gitr_ref TYPE string;
DEFINE FIELD generation ON gitr_ref TYPE int;
DEFINE TABLE artifact SCHEMAFULL;
DEFINE FIELD payload ON artifact TYPE string;
DEFINE TABLE receipt SCHEMAFULL;
DEFINE FIELD status ON receipt TYPE string;
DEFINE FIELD artifact ON receipt TYPE option<record<artifact>>;
DEFINE TABLE marker SCHEMAFULL;
DEFINE FIELD value ON marker TYPE string;
DEFINE TABLE code_unit SCHEMAFULL;
DEFINE FIELD name ON code_unit TYPE string;
DEFINE FIELD body ON code_unit TYPE string;
DEFINE FIELD embedding ON code_unit TYPE array<float>;
DEFINE TABLE calls TYPE RELATION IN code_unit OUT code_unit SCHEMAFULL;
DEFINE ANALYZER code_terms TOKENIZERS blank FILTERS lowercase;
DEFINE INDEX lexical ON code_unit FIELDS body FULLTEXT ANALYZER code_terms BM25;
DEFINE INDEX vectors ON code_unit FIELDS embedding HNSW DIMENSION 3 DIST COSINE;
