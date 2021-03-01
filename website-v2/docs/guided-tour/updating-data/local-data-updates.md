---
id: local-data-updates
title: Local Data Updates
slug: /guided-tour/updating-data/local-data-updates/
---

import DocsRating from '../../../src/core/DocsRating';
import {OssOnly, FbInternalOnly} from 'internaldocs-fb-helpers';
import FbLocalDataUpdatesFlow from './fb/FbLocalDataUpdatesFlow.md';

## Local data updates

There are a couple of APIs that Relay provides in order to make purely local updates to the Relay store (i.e. updates not tied to a server operation).

Note that local data updates can be made both on [client-only data](../client-only-data/), or on regular data that was fetched from the server via an operation.

### commitLocalUpdate

To make updates using an [`updater`](../graphql-mutations/#updater-functions) function, you can use the `commitLocalUpdate` API:

```js
import type {Environment} from 'react-relay';

const {commitLocalUpdate, graphql} = require('react-relay');

function commitCommentCreateLocally(
  environment: Environment,
  feedbackID: string,
) {
  return commitLocalUpdate(environment, store => {
    const feedbackRecord = store.get(feedbackID);
    const connectionRecord = ConnectionHandler.getConnection(
      userRecord,
      'CommentsComponent_comments_connection',
    );

    // Create a new local Comment from scratch
    const id = `client:new_comment:${randomID()}`;
    const newCommentRecord = store.create(id, 'Comment');

    // ... update new comment with content

    // Create new edge from scratch
    const newEdge = ConnectionHandler.createEdge(
      store,
      connectionRecord,
      newCommentRecord,
      'CommentEdge' /* GraphQl Type for edge */,
    );

    // Add edge to the end of the connection
    ConnectionHandler.insertEdgeAfter(connectionRecord, newEdge);
  });
}

module.exports = {commit: commitCommentCreateLocally};
```

* `commitLocalUpdate` update simply takes an environment and an updater function.
    * `updater` takes a *`store`* argument, which is an instance of a [`RecordSourceSelectorProxy`](../../../api-reference/store/);  this interface allows you to *imperatively* write and read data directly to and from the Relay store. This means that you have full control over how to update the store: you can *create entirely new records*, or *update or delete existing ones*.
* In our specific example, we're adding a new comment to our local store when. Specifically, we're adding a new item to a connection; for more details on the specifics of how that works, check out our [Adding and Removing Items from a Connection](../../list-data/adding-and-removing-items/) section.
* Note that any local data updates will automatically cause components subscribed to the data to be notified of the change and re-render.

### commitPayload

`commitPayload` takes an `OperationDescriptor` and the payload for the query, and writes it to the Relay Store. The payload will be resolved like a normal server response for a query, and will also resolve Data Driven Dependencies that are passed as `JSResource`, `requireDefer`, etc.

```js
import type {FooQueryRawResponse} from 'FooQuery.graphql'

const {createOperationDescriptor} = require('relay-runtime');

const operationDescriptor = createOperationDescriptor(FooQuery, {
  id: 'an-id',
  otherVariable: 'value',
});

const payload: FooQueryRawResponse = {...};

environment.commitPayload(operation, payload);
```

* An `OperationDescriptor` can be created by `createOperationDescriptor`; it takes the query and the query variables.
* The payload can be typed using the Flow type generated by adding  `@raw_response_type` to the query.
* Note that any local data updates will automatically cause components subscribed to the data to be notified of the change and re-render.

<FbLocalDataUpdatesFlow />


<DocsRating />