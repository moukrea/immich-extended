import {
  createContext,
  createResource,
  useContext,
  type ParentComponent,
  type Resource,
} from "solid-js";
import { fetchPeople, type MePerson } from "../lib/api";

// Shared people resource for the rule builder's four people multi-selects.
// Without this, each multi-select would create its own `createResource` and
// the proxy `/api/v1/me/people` would be hit N times per page load.
//
// The `noImmichKey` flag lets every consumer render the "Connect your Immich
// account at Settings" CTA without re-fetching the resource themselves.

export interface PeopleListing {
  people: MePerson[];
  noImmichKey: boolean;
}

export const PeopleContext = createContext<
  Resource<PeopleListing> | undefined
>();

export const PeopleProvider: ParentComponent = (props) => {
  const [people] = createResource<PeopleListing>(async () => {
    const result = await fetchPeople();
    if (result.ok) {
      return { people: result.data, noImmichKey: false };
    }
    if (result.noImmichKey) {
      return { people: [], noImmichKey: true };
    }
    return { people: [], noImmichKey: false };
  });
  return (
    <PeopleContext.Provider value={people}>
      {props.children}
    </PeopleContext.Provider>
  );
};

export function usePeople(): Resource<PeopleListing> | undefined {
  return useContext(PeopleContext);
}
