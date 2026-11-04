-- The Feature a runner created for an accepted `start_feature`, as its first
-- `POST /git-token` for that request named it (decision 71). Set once, by a
-- conditional UPDATE, and compared with every later fetch: a token is minted
-- for the run that start created and for no other Feature the same runner
-- holds. NULL on every other request, and on a start no token was asked for.
ALTER TABLE instance_requests
  ADD COLUMN git_token_feature_id TEXT;
