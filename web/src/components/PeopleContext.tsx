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

export const PeopleContext = createContext<Resource<MePerson[]> | undefined>();

export const PeopleProvider: ParentComponent = (props) => {
  const [people] = createResource<MePerson[]>(async () => {
    const result = await fetchPeople();
    return result.ok ? result.data : [];
  });
  return (
    <PeopleContext.Provider value={people}>
      {props.children}
    </PeopleContext.Provider>
  );
};

export function usePeople(): Resource<MePerson[]> | undefined {
  return useContext(PeopleContext);
}
