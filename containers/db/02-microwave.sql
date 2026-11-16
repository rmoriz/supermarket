-- A second database for the Infra Server. Runs once, on an empty cluster.
CREATE ROLE microwave LOGIN PASSWORD 'microwave';
CREATE DATABASE microwave OWNER microwave;
